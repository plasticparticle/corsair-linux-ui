mod bragi;
mod input;
mod model;
mod rgb;

use input::{DriverStatus, InputDriver};
use model::{ButtonDefinition, Config, Device, Profile};
use rgb::{RgbAdapter, RgbStatus};
use serde::Serialize;
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};

#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

fn show_main_window(app: &tauri::AppHandle) {
    show_main_window_at(app, app.cursor_position().ok());
}

fn show_main_window_at(app: &tauri::AppHandle, activation_point: Option<PhysicalPosition<f64>>) {
    if let Some(window) = app.get_webview_window("main") {
        move_to_activation_monitor(&window, activation_point);
        #[cfg(target_os = "linux")]
        let _ = window.set_visible_on_all_workspaces(true);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        #[cfg(target_os = "linux")]
        let _ = window.set_visible_on_all_workspaces(false);
    }
}

fn point_inside_monitor(
    point: PhysicalPosition<f64>,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
) -> bool {
    let right = f64::from(position.x) + f64::from(size.width);
    let bottom = f64::from(position.y) + f64::from(size.height);
    point.x >= f64::from(position.x)
        && point.x < right
        && point.y >= f64::from(position.y)
        && point.y < bottom
}

fn centered_window_position(
    monitor_position: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
    window_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let available_x = monitor_size.width.saturating_sub(window_size.width) / 2;
    let available_y = monitor_size.height.saturating_sub(window_size.height) / 2;
    PhysicalPosition::new(
        monitor_position.x.saturating_add(available_x as i32),
        monitor_position.y.saturating_add(available_y as i32),
    )
}

fn move_to_activation_monitor(
    window: &tauri::WebviewWindow,
    activation_point: Option<PhysicalPosition<f64>>,
) {
    let Some(point) = activation_point else {
        return;
    };
    let Ok(monitors) = window.available_monitors() else {
        return;
    };
    let Some(target) = monitors
        .iter()
        .find(|monitor| point_inside_monitor(point, *monitor.position(), *monitor.size()))
    else {
        return;
    };
    let already_on_target = window
        .current_monitor()
        .ok()
        .flatten()
        .is_some_and(|current| current.position() == target.position());
    if already_on_target {
        return;
    }

    let was_maximized = window.is_maximized().unwrap_or(false);
    if was_maximized {
        let _ = window.unmaximize();
    }
    if let Ok(window_size) = window.outer_size() {
        let position = centered_window_position(*target.position(), *target.size(), window_size);
        let _ = window.set_position(position);
    }
    if was_maximized {
        let _ = window.maximize();
    }
}

#[cfg(target_os = "linux")]
struct LinuxTray {
    app: tauri::AppHandle,
    icon: ksni::Icon,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "corsair-control".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn title(&self) -> String {
        "Corsair Control".into()
    }

    fn icon_name(&self) -> String {
        "corsair-control".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: "corsair-control".into(),
            icon_pixmap: vec![self.icon.clone()],
            title: "Corsair Control".into(),
            description: "Click to show mouse controls".into(),
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        let point = if x == 0 && y == 0 {
            self.app.cursor_position().ok()
        } else {
            Some(PhysicalPosition::new(f64::from(x), f64::from(y)))
        };
        show_main_window_at(&self.app, point);
    }

    fn secondary_activate(&mut self, x: i32, y: i32) {
        self.activate(x, y);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open Corsair Control".into(),
                icon_name: "window-restore".into(),
                activate: Box::new(|tray: &mut Self| show_main_window(&tray.app)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.app.exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(target_os = "linux")]
fn linux_tray_icon(image: &tauri::image::Image<'_>) -> ksni::Icon {
    let mut data = image.rgba().to_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }
    ksni::Icon {
        width: image.width() as i32,
        height: image.height() as i32,
        data,
    }
}

struct AppState {
    config: Arc<Mutex<Config>>,
    devices: Mutex<Vec<Device>>,
    driver: InputDriver,
    rgb: Mutex<RgbAdapter>,
    config_path: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    config: Config,
    devices: Vec<Device>,
    buttons: Vec<ButtonDefinition>,
    driver: DriverStatus,
    rgb: RgbStatus,
}

#[tauri::command]
fn get_state(state: tauri::State<AppState>) -> Snapshot {
    Snapshot {
        config: state.config.lock().unwrap().clone(),
        devices: state.devices.lock().unwrap().clone(),
        buttons: model::buttons(),
        driver: state.driver.status(),
        rgb: state.rgb.lock().unwrap().status.clone(),
    }
}

#[tauri::command]
fn save_profile(profile: Profile, state: tauri::State<AppState>) -> Result<(), String> {
    Config::validate_profile(&profile)?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let is_active = config.active_profile == profile.id;
    let selected_dpi = profile.dpi_stages[profile.active_dpi];
    let Some(existing) = config
        .profiles
        .iter_mut()
        .find(|item| item.id == profile.id)
    else {
        return Err("Profile not found".into());
    };
    *existing = profile;
    model::save_config(&state.config_path, &config)?;
    drop(config);
    if is_active {
        state.driver.apply_dpi(selected_dpi)?;
    }
    Ok(())
}

#[tauri::command]
fn create_profile(profile: Profile, state: tauri::State<AppState>) -> Result<(), String> {
    Config::validate_profile(&profile)?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    if config.profiles.iter().any(|item| item.id == profile.id) {
        return Err("Profile already exists".into());
    }
    config.profiles.push(profile);
    model::save_config(&state.config_path, &config)
}

#[tauri::command]
fn activate_profile(profile_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    if !config.profiles.iter().any(|item| item.id == profile_id) {
        return Err("Profile not found".into());
    }
    config.active_profile = profile_id;
    let selected_dpi = config
        .active()
        .dpi_stages
        .get(config.active().active_dpi)
        .copied()
        .ok_or("Active DPI stage is missing")?;
    model::save_config(&state.config_path, &config)?;
    drop(config);
    state.driver.apply_dpi(selected_dpi)
}

#[tauri::command]
fn learn_button(button_id: String, state: tauri::State<AppState>) {
    state.driver.learn(button_id);
}

#[tauri::command]
fn rescan_devices(state: tauri::State<AppState>) -> Vec<Device> {
    let devices = model::discover_devices();
    *state.devices.lock().unwrap() = devices.clone();
    *state.rgb.lock().unwrap() = RgbAdapter::probe(&devices, state.driver.bragi_control());
    devices
}

#[tauri::command]
fn apply_rgb(state: tauri::State<AppState>) -> Result<(), String> {
    let lighting = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .active()
        .lighting
        .clone();
    state
        .rgb
        .lock()
        .map_err(|e| e.to_string())?
        .apply(&lighting)
}

#[tauri::command]
fn window_minimize(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
fn window_start_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
fn window_toggle_maximize(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize()
    } else {
        window.maximize()
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn window_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
const AUTOSTART_DESKTOP_ENTRY: &str = "[Desktop Entry]\n\
Type=Application\n\
Name=Corsair Control\n\
Comment=Start Corsair Control in the system tray\n\
Exec=corsair-control --background\n\
Icon=corsair-control\n\
Terminal=false\n\
StartupNotify=false\n\
X-GNOME-Autostart-enabled=true\n\
X-Corsair-Control-Autostart=true\n";

#[cfg(target_os = "linux")]
fn linux_config_home(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, String> {
    if let Some(value) = xdg_config_home {
        let path = PathBuf::from(value);
        if !value.is_empty() && path.is_absolute() {
            return Ok(path);
        }
    }
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| path.join(".config"))
        .ok_or_else(|| "Cannot find your Linux configuration directory".to_string())
}

#[cfg(target_os = "linux")]
fn autostart_path() -> Result<PathBuf, String> {
    linux_config_home(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
    .map(|path| path.join("autostart/corsair-control.desktop"))
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn get_autostart() -> Result<bool, String> {
    let path = autostart_path()?;
    match fs::read_to_string(path) {
        Ok(entry) => Ok(entry
            .lines()
            .any(|line| line.trim() == "X-Corsair-Control-Autostart=true")
            && !entry.lines().any(|line| line.trim() == "Hidden=true")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot read the login setting: {error}")),
    }
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<bool, String> {
    let path = autostart_path()?;
    if !enabled {
        return match fs::remove_file(path) {
            Ok(()) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("Cannot disable start on login: {error}")),
        };
    }

    let parent = path
        .parent()
        .ok_or_else(|| "The login setting path has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create the autostart directory: {error}"))?;
    let temporary = path.with_extension(format!("desktop.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Cannot prepare the login setting: {error}"))?;
    file.write_all(AUTOSTART_DESKTOP_ENTRY.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Cannot write the login setting: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Cannot enable start on login: {error}"))?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn get_autostart() -> Result<bool, String> {
    Err("Start on login is currently available on Linux only".into())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn set_autostart(_enabled: bool) -> Result<bool, String> {
    Err("Start on login is currently available on Linux only".into())
}

fn main() {
    let start_in_background = std::env::args_os().any(|argument| argument == "--background");
    let config_path = model::config_path();
    let devices = model::discover_devices();
    let mut loaded_config = model::load_config(&config_path);
    if model::apply_device_defaults(&mut loaded_config, &devices) {
        let _ = model::save_config(&config_path, &loaded_config);
    }
    let config = Arc::new(Mutex::new(loaded_config));
    let nodes = devices
        .iter()
        .flat_map(|device| device.input_nodes.clone())
        .collect();
    let (physical_tx, physical_rx) = mpsc::channel();
    let driver = InputDriver::start(nodes, config.clone(), physical_tx);
    let bragi_rgb = driver.bragi_control();
    let rgb_devices = devices.clone();
    let rgb = RgbAdapter::pending(&devices);
    let state = AppState {
        config,
        devices: Mutex::new(devices),
        driver,
        rgb: Mutex::new(rgb),
        config_path,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                show_main_window(app);
            },
        ))
        .manage(state)
        .setup(move |app| {
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("application icon is missing")?;

            #[cfg(target_os = "linux")]
            {
                use ksni::blocking::TrayMethods;
                LinuxTray {
                    app: app.handle().clone(),
                    icon: linux_tray_icon(&icon),
                }
                .assume_sni_available(true)
                .spawn()
                .map_err(|error| format!("failed to create Linux tray icon: {error:?}"))?;
            }

            #[cfg(not(target_os = "linux"))]
            {
                let open =
                    MenuItem::with_id(app, "open", "Open Corsair Control", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&open, &quit])?;
                TrayIconBuilder::with_id("corsair-control")
                    .icon(icon)
                    .tooltip("Corsair Control")
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "open" => show_main_window(app),
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main_window(tray.app_handle());
                        }
                    })
                    .build(app)?;
            }

            let app_handle = app.handle().clone();
            thread::spawn(move || {
                for input in physical_rx {
                    if input.pressed {
                        if let Ok(rgb) = app_handle.state::<AppState>().rgb.lock() {
                            rgb.physical_input();
                        }
                    }
                    let _ = app_handle.emit("physical-input", input);
                }
            });
            let rgb_handle = app.handle().clone();
            thread::spawn(move || {
                let adapter = RgbAdapter::probe(&rgb_devices, bragi_rgb);
                *rgb_handle.state::<AppState>().rgb.lock().unwrap() = adapter;
            });
            if !start_in_background {
                show_main_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_profile,
            create_profile,
            activate_profile,
            learn_button,
            rescan_devices,
            apply_rgb,
            window_start_drag,
            window_minimize,
            window_toggle_maximize,
            window_close,
            get_autostart,
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Corsair Control");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_config_home_precedes_home_for_autostart() {
        assert_eq!(
            linux_config_home(
                Some(OsStr::new("/tmp/corsair-xdg")),
                Some(OsStr::new("/tmp/corsair-home"))
            ),
            Ok(PathBuf::from("/tmp/corsair-xdg"))
        );
        assert_eq!(
            linux_config_home(
                Some(OsStr::new("relative-path")),
                Some(OsStr::new("/tmp/corsair-home"))
            ),
            Ok(PathBuf::from("/tmp/corsair-home/.config"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn autostart_entry_launches_the_tray_in_background() {
        assert!(AUTOSTART_DESKTOP_ENTRY.contains("Exec=corsair-control --background"));
        assert!(AUTOSTART_DESKTOP_ENTRY.contains("X-Corsair-Control-Autostart=true"));
    }

    #[test]
    fn monitor_hit_test_supports_negative_desktop_coordinates() {
        let position = PhysicalPosition::new(-1_920, 0);
        let size = PhysicalSize::new(1_920, 1_080);
        assert!(point_inside_monitor(
            PhysicalPosition::new(-20.0, 400.0),
            position,
            size
        ));
        assert!(!point_inside_monitor(
            PhysicalPosition::new(20.0, 400.0),
            position,
            size
        ));
    }

    #[test]
    fn centers_window_on_activation_monitor() {
        assert_eq!(
            centered_window_position(
                PhysicalPosition::new(1_920, 0),
                PhysicalSize::new(2_560, 1_440),
                PhysicalSize::new(1_280, 820),
            ),
            PhysicalPosition::new(2_560, 310)
        );
    }

    #[test]
    fn oversized_window_stays_at_monitor_origin() {
        assert_eq!(
            centered_window_position(
                PhysicalPosition::new(-1_280, -200),
                PhysicalSize::new(1_280, 720),
                PhysicalSize::new(1_600, 900),
            ),
            PhysicalPosition::new(-1_280, -200)
        );
    }
}
