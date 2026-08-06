use crate::{
    bragi::{self, BragiControl},
    model::{Config, Mapping},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    mem,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::Sender,
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UInputUserDev {
    name: [u8; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

impl Default for UInputUserDev {
    fn default() -> Self {
        Self {
            name: [0; 80],
            id: InputId::default(),
            ff_effects_max: 0,
            absmax: [0; 64],
            absmin: [0; 64],
            absfuzz: [0; 64],
            absflat: [0; 64],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

struct UInput {
    file: File,
}

impl UInput {
    fn new() -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")?;
        let fd = file.as_raw_fd();
        unsafe {
            for event_type in [EV_SYN, EV_KEY, EV_REL] {
                if libc::ioctl(fd, UI_SET_EVBIT, event_type as libc::c_ulong) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            for code in 0..0x300_u32 {
                libc::ioctl(fd, UI_SET_KEYBIT, code as libc::c_ulong);
            }
            for code in 0..0x10_u32 {
                libc::ioctl(fd, UI_SET_RELBIT, code as libc::c_ulong);
            }
        }
        let mut setup = UInputUserDev::default();
        let name = b"Corsair Control Virtual Input";
        setup.name[..name.len()].copy_from_slice(name);
        setup.id = InputId {
            bustype: 0x06,
            vendor: 0x1b1c,
            product: 0xc001,
            version: 1,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&setup as *const UInputUserDev).cast::<u8>(),
                mem::size_of::<UInputUserDev>(),
            )
        };
        file.write_all(bytes)?;
        if unsafe { libc::ioctl(fd, UI_DEV_CREATE) } < 0 {
            return Err(io::Error::last_os_error());
        }
        thread::sleep(Duration::from_millis(80));
        Ok(Self { file })
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> io::Result<()> {
        let mut event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        unsafe {
            libc::gettimeofday(&mut event.time, std::ptr::null_mut());
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const InputEvent).cast::<u8>(),
                mem::size_of::<InputEvent>(),
            )
        };
        self.file.write_all(bytes)
    }

    fn sync(&mut self) {
        let _ = self.emit(EV_SYN, 0, 0);
    }
}

impl Drop for UInput {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverStatus {
    pub running: bool,
    pub permission_required: bool,
    pub input_nodes: Vec<String>,
    pub error: String,
    pub learn_button: String,
    pub last_capture: Option<Capture>,
    pub active_sources: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub button_id: String,
    pub source: String,
    pub node: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalInput {
    pub source: String,
    pub pressed: bool,
    pub pulse: bool,
}

pub struct InputDriver {
    status: Arc<Mutex<DriverStatus>>,
    activity: Arc<Mutex<BTreeMap<String, Option<Instant>>>>,
    stop: Arc<AtomicBool>,
    _bragi_control: Option<BragiControl>,
}

#[derive(Clone)]
struct ReaderContext {
    status: Arc<Mutex<DriverStatus>>,
    activity: Arc<Mutex<BTreeMap<String, Option<Instant>>>>,
    stop: Arc<AtomicBool>,
    output: Arc<Mutex<UInput>>,
    config: Arc<Mutex<Config>>,
    physical_events: Sender<PhysicalInput>,
    active_readers: Arc<AtomicUsize>,
}

fn event_source(type_: u16, code: u16, value: i32) -> String {
    if type_ == EV_REL {
        format!("EV_REL:{code}:{}", if value > 0 { 1 } else { -1 })
    } else {
        format!("EV_KEY:{code}")
    }
}

fn update_activity(
    activity: &mut BTreeMap<String, Option<Instant>>,
    event: &InputEvent,
    now: Instant,
) {
    if event.type_ == EV_KEY {
        let source = event_source(event.type_, event.code, event.value);
        match event.value {
            1 => {
                activity.insert(source, None);
            }
            0 => {
                // Keep a completed click visible long enough for the polling
                // fallback. Native window events are faster, but polling must
                // remain reliable when WebKit's event bridge is unavailable.
                activity.insert(source, Some(now + Duration::from_millis(500)));
            }
            _ => {}
        }
    } else if event.type_ == EV_REL && event.value != 0 {
        activity.insert(
            event_source(event.type_, event.code, event.value),
            Some(now + Duration::from_millis(500)),
        );
    }
}

fn physical_input(event: &InputEvent) -> Option<PhysicalInput> {
    if event.type_ == EV_KEY && matches!(event.value, 0 | 1) {
        Some(PhysicalInput {
            source: event_source(event.type_, event.code, event.value),
            pressed: event.value == 1,
            pulse: false,
        })
    } else if event.type_ == EV_REL && event.value != 0 && matches!(event.code, 6 | 8 | 11 | 12) {
        Some(PhysicalInput {
            source: event_source(event.type_, event.code, event.value),
            pressed: true,
            pulse: true,
        })
    } else {
        None
    }
}

fn key_code(name: &str) -> Option<u16> {
    Some(match name.trim().to_ascii_uppercase().as_str() {
        "ESC" => 1,
        "BACKSPACE" => 14,
        "TAB" => 15,
        "ENTER" => 28,
        "CTRL" => 29,
        "SHIFT" => 42,
        "ALT" => 56,
        "SPACE" => 57,
        "CAPSLOCK" => 58,
        "F1" => 59,
        "F2" => 60,
        "F3" => 61,
        "F4" => 62,
        "F5" => 63,
        "F6" => 64,
        "F7" => 65,
        "F8" => 66,
        "F9" => 67,
        "F10" => 68,
        "F11" => 87,
        "F12" => 88,
        "F13" => 183,
        "F14" => 184,
        "F15" => 185,
        "F16" => 186,
        "F17" => 187,
        "F18" => 188,
        "F19" => 189,
        "F20" => 190,
        "F21" => 191,
        "F22" => 192,
        "F23" => 193,
        "F24" => 194,
        "HOME" => 102,
        "UP" => 103,
        "PAGEUP" => 104,
        "LEFT" => 105,
        "RIGHT" => 106,
        "END" => 107,
        "DOWN" => 108,
        "PAGEDOWN" => 109,
        "INSERT" => 110,
        "DELETE" => 111,
        "MUTE" => 113,
        "VOLUMEDOWN" => 114,
        "VOLUMEUP" => 115,
        "META" | "SUPER" => 125,
        "STOP" => 128,
        "CALC" => 140,
        "MAIL" => 155,
        "BACK" => 158,
        "FORWARD" => 159,
        "NEXTSONG" => 163,
        "PLAYPAUSE" => 164,
        "PREVIOUSSONG" => 165,
        "BTN_LEFT" => 272,
        "BTN_RIGHT" => 273,
        "BTN_MIDDLE" => 274,
        "BTN_SIDE" => 275,
        "BTN_EXTRA" => 276,
        "BTN_FORWARD" => 277,
        "BTN_BACK" => 278,
        one if one.len() == 1 => {
            let byte = one.as_bytes()[0];
            match byte {
                b'A'..=b'L' => {
                    [30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38][(byte - b'A') as usize]
                }
                b'M'..=b'Z' => {
                    [50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44][(byte - b'M') as usize]
                }
                b'1'..=b'9' => (byte - b'1' + 2) as u16,
                b'0' => 11,
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn find_mapping(config: &Config, source: &str) -> Option<(String, Mapping)> {
    let profile = config.active();
    profile
        .sources
        .iter()
        .find(|(_, candidate)| candidate.as_str() == source)
        .and_then(|(id, _)| {
            profile
                .mappings
                .get(id)
                .cloned()
                .map(|mapping| (id.clone(), mapping))
        })
}

fn internal_action(config: &mut Config, action: &str, direction: &str) {
    if action == "profile" && !config.profiles.is_empty() {
        let index = config
            .profiles
            .iter()
            .position(|p| p.id == config.active_profile)
            .unwrap_or(0);
        let next = if direction == "previous" {
            (index + config.profiles.len() - 1) % config.profiles.len()
        } else {
            (index + 1) % config.profiles.len()
        };
        config.active_profile = config.profiles[next].id.clone();
    } else if action == "dpi" {
        let active_id = config.active_profile.clone();
        if let Some(profile) = config.profiles.iter_mut().find(|p| p.id == active_id) {
            let len = profile.dpi_stages.len();
            if len > 0 {
                profile.active_dpi = if direction == "previous" {
                    (profile.active_dpi + len - 1) % len
                } else {
                    (profile.active_dpi + 1) % len
                };
            }
        }
    }
}

fn process_source_event(
    output: &mut UInput,
    config: &Arc<Mutex<Config>>,
    source: &str,
    type_: u16,
    code: u16,
    value: i32,
) {
    let mapping = config
        .lock()
        .ok()
        .and_then(|cfg| find_mapping(&cfg, source));
    let Some((_id, mapping)) = mapping else {
        let _ = output.emit(type_, code, value);
        return;
    };
    match mapping.action.as_str() {
        "passthrough" => {
            let _ = output.emit(type_, code, value);
        }
        "disabled" => {}
        "dpi" | "profile" if value > 0 => {
            if let Ok(mut cfg) = config.lock() {
                internal_action(&mut cfg, &mapping.action, &mapping.value);
            }
        }
        "key" | "media" => {
            if let Some(code) = key_code(&mapping.value) {
                let value = if type_ == EV_REL { 1 } else { value };
                let _ = output.emit(EV_KEY, code, value);
                if type_ == EV_REL {
                    let _ = output.emit(EV_KEY, code, 0);
                }
            }
        }
        "shortcut" if value > 0 => {
            let keys: Vec<u16> = mapping.value.split('+').filter_map(key_code).collect();
            for code in &keys {
                let _ = output.emit(EV_KEY, *code, 1);
            }
            output.sync();
            for code in keys.iter().rev() {
                let _ = output.emit(EV_KEY, *code, 0);
            }
            output.sync();
        }
        _ => {}
    }
}

fn process_event(output: &mut UInput, config: &Arc<Mutex<Config>>, event: InputEvent) {
    if event.type_ == EV_SYN {
        let _ = output.emit(event.type_, event.code, event.value);
        return;
    }
    if event.type_ != EV_KEY && event.type_ != EV_REL {
        return;
    }
    let source = event_source(event.type_, event.code, event.value);
    process_source_event(
        output,
        config,
        &source,
        event.type_,
        event.code,
        event.value,
    );
}

impl InputDriver {
    pub fn start(
        nodes: Vec<String>,
        config: Arc<Mutex<Config>>,
        physical_events: Sender<PhysicalInput>,
    ) -> Self {
        let status = Arc::new(Mutex::new(DriverStatus {
            input_nodes: nodes.clone(),
            ..Default::default()
        }));
        let activity = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let active_readers = Arc::new(AtomicUsize::new(0));
        if nodes.is_empty() {
            status.lock().unwrap().error =
                "No Corsair input nodes found. Reconnect the device after installing udev rules."
                    .into();
            return Self {
                status,
                activity,
                stop,
                _bragi_control: None,
            };
        }
        let output = match UInput::new() {
            Ok(value) => Arc::new(Mutex::new(value)),
            Err(error) => {
                let mut current = status.lock().unwrap();
                current.permission_required = error.kind() == io::ErrorKind::PermissionDenied;
                current.error = format!("Cannot open /dev/uinput: {error}");
                drop(current);
                return Self {
                    status,
                    activity,
                    stop,
                    _bragi_control: None,
                };
            }
        };
        let bragi_control = match BragiControl::activate() {
            Ok(control) => Some(control),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                let mut current = status.lock().unwrap();
                current.permission_required = error.kind() == io::ErrorKind::PermissionDenied;
                current.error = format!("Cannot enable Wireless SE controls: {error}");
                None
            }
        };
        let mut started = 0;
        status.lock().unwrap().error = "Starting physical input readers…".into();
        for node in nodes {
            if !std::path::Path::new(&node).exists() {
                continue;
            }
            started += 1;
            let context = ReaderContext {
                status: status.clone(),
                activity: activity.clone(),
                stop: stop.clone(),
                output: output.clone(),
                config: config.clone(),
                physical_events: physical_events.clone(),
                active_readers: active_readers.clone(),
            };
            thread::spawn(move || read_device(&node, context));
        }
        if let Some(control) = &bragi_control {
            let node = control.input_path().to_string_lossy().into_owned();
            status.lock().unwrap().input_nodes.push(node.clone());
            started += 1;
            let context = ReaderContext {
                status: status.clone(),
                activity: activity.clone(),
                stop: stop.clone(),
                output: output.clone(),
                config: config.clone(),
                physical_events: physical_events.clone(),
                active_readers: active_readers.clone(),
            };
            thread::spawn(move || read_bragi_input(&node, context));
        }
        let mut current = status.lock().unwrap();
        if started == 0 {
            current.error = "Input event nodes disappeared; use Rescan after reconnecting.".into();
        }
        drop(current);
        Self {
            status,
            activity,
            stop,
            _bragi_control: bragi_control,
        }
    }

    pub fn status(&self) -> DriverStatus {
        let now = Instant::now();
        let active_sources = {
            let mut activity = self.activity.lock().unwrap();
            activity.retain(|_, expires| match expires {
                Some(deadline) => *deadline > now,
                None => true,
            });
            activity.keys().cloned().collect()
        };
        let mut status = self.status.lock().unwrap().clone();
        status.active_sources = active_sources;
        status
    }

    pub fn learn(&self, button_id: String) {
        let mut status = self.status.lock().unwrap();
        status.learn_button = button_id;
        status.last_capture = None;
    }
}

fn read_bragi_input(node: &str, context: ReaderContext) {
    let ReaderContext {
        status,
        activity,
        stop,
        output,
        config,
        physical_events,
        active_readers,
    } = context;
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(node)
    {
        Ok(file) => file,
        Err(error) => {
            let mut current = status.lock().unwrap();
            current.permission_required |= error.kind() == io::ErrorKind::PermissionDenied;
            current.error = format!("Cannot read Wireless SE controls from {node}: {error}");
            return;
        }
    };
    active_readers.fetch_add(1, Ordering::Relaxed);
    {
        let mut current = status.lock().unwrap();
        current.running = true;
        current.permission_required = false;
        current.error.clear();
    }

    let mut previous = 0_u16;
    let mut report = [0_u8; 64];
    while !stop.load(Ordering::Relaxed) {
        match file.read(&mut report) {
            Ok(64) => {}
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => {
                status.lock().unwrap().error =
                    format!("Wireless SE control channel disconnected: {error}");
                break;
            }
        }
        let Some(mask) = bragi::button_mask(&report) else {
            continue;
        };
        let changed = previous ^ mask;
        for bit in 3_u8..16 {
            let flag = 1_u16 << bit;
            if changed & flag == 0 {
                continue;
            }
            let pressed = mask & flag != 0;
            let source = bragi::source_for_bit(bit);
            let _ = physical_events.send(PhysicalInput {
                source: source.clone(),
                pressed,
                pulse: false,
            });
            if let Ok(mut active) = activity.lock() {
                active.insert(
                    source.clone(),
                    if pressed {
                        None
                    } else {
                        Some(Instant::now() + Duration::from_millis(500))
                    },
                );
            }
            if pressed {
                let mut current = status.lock().unwrap();
                if !current.learn_button.is_empty() {
                    let button_id = mem::take(&mut current.learn_button);
                    current.last_capture = Some(Capture {
                        button_id,
                        source: source.clone(),
                        node: node.into(),
                    });
                }
            }
            if let Ok(mut virtual_device) = output.lock() {
                process_source_event(
                    &mut virtual_device,
                    &config,
                    &source,
                    EV_KEY,
                    bragi::linux_code_for_bit(bit),
                    i32::from(pressed),
                );
                virtual_device.sync();
            }
        }
        previous = mask;
    }
    if active_readers.fetch_sub(1, Ordering::Relaxed) == 1 {
        status.lock().unwrap().running = false;
    }
}

impl Drop for InputDriver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn read_device(node: &str, context: ReaderContext) {
    let ReaderContext {
        status,
        activity,
        stop,
        output,
        config,
        physical_events,
        active_readers,
    } = context;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(node)
    {
        Ok(file) => file,
        Err(error) => {
            let mut s = status.lock().unwrap();
            s.permission_required |= error.kind() == io::ErrorKind::PermissionDenied;
            s.error = format!("Cannot read {node}: {error}");
            return;
        }
    };
    let fd = file.as_raw_fd();
    if unsafe { libc::ioctl(fd, EVIOCGRAB, 1) } < 0 {
        let error = io::Error::last_os_error();
        let mut s = status.lock().unwrap();
        s.permission_required |= error.kind() == io::ErrorKind::PermissionDenied;
        s.error = format!("Cannot grab {node}: {error}");
        return;
    }
    active_readers.fetch_add(1, Ordering::Relaxed);
    {
        let mut current = status.lock().unwrap();
        current.running = true;
        current.permission_required = false;
        current.error.clear();
    }
    let mut events = [InputEvent {
        time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        type_: 0,
        code: 0,
        value: 0,
    }; 32];
    while !stop.load(Ordering::Relaxed) {
        let read = unsafe { libc::read(fd, events.as_mut_ptr().cast(), mem::size_of_val(&events)) };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            status.lock().unwrap().error = format!("Input disconnected: {error}");
            break;
        }
        let count = read as usize / mem::size_of::<InputEvent>();
        for event in &events[..count] {
            if let Some(physical) = physical_input(event) {
                let _ = physical_events.send(physical);
            }
            if let Ok(mut active) = activity.lock() {
                update_activity(&mut active, event, Instant::now());
            }
            if event.value != 0 && (event.type_ == EV_KEY || event.type_ == EV_REL) {
                let mut current = status.lock().unwrap();
                if !current.learn_button.is_empty() {
                    let button_id = mem::take(&mut current.learn_button);
                    current.last_capture = Some(Capture {
                        button_id,
                        source: event_source(event.type_, event.code, event.value),
                        node: node.into(),
                    });
                }
            }
            if let Ok(mut virtual_device) = output.lock() {
                process_event(&mut virtual_device, &config, *event);
            }
        }
    }
    unsafe {
        libc::ioctl(fd, EVIOCGRAB, 0);
    }
    if active_readers.fetch_sub(1, Ordering::Relaxed) == 1 {
        status.lock().unwrap().running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_sources_include_wheel_direction() {
        assert_eq!(event_source(EV_REL, 8, -1), "EV_REL:8:-1");
        assert_eq!(event_source(EV_KEY, 277, 1), "EV_KEY:277");
    }
    #[test]
    fn parses_shortcut_keys() {
        assert_eq!(key_code("ctrl"), Some(29));
        assert_eq!(key_code("T"), Some(20));
        assert_eq!(key_code("F13"), Some(183));
        assert_eq!(key_code("F24"), Some(194));
    }

    #[test]
    fn physical_key_activity_tracks_holds_and_latches_releases() {
        let mut activity = BTreeMap::new();
        let now = Instant::now();
        let mut event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: EV_KEY,
            code: 272,
            value: 1,
        };
        update_activity(&mut activity, &event, now);
        assert_eq!(activity.get("EV_KEY:272"), Some(&None));
        event.value = 0;
        update_activity(&mut activity, &event, now);
        assert!(activity["EV_KEY:272"].is_some());
    }

    #[test]
    fn physical_events_ignore_pointer_motion() {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: EV_REL,
            code: 0,
            value: 14,
        };
        assert!(physical_input(&event).is_none());
    }
}
