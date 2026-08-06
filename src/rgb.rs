use crate::{
    bragi::{BragiControl, RGB_LED_COUNT},
    model::{Device, Lighting},
};
use serde::Serialize;
use std::{
    f32::consts::TAU,
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FRAME_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbStatus {
    pub available: bool,
    pub backend: String,
    pub device_name: String,
    pub reason: String,
    pub last_applied: Option<u64>,
}

enum RgbBackend {
    None,
    OpenRgb { selector: String },
    Bragi { control: Arc<Mutex<BragiControl>> },
}

pub struct RgbAdapter {
    pub status: RgbStatus,
    backend: RgbBackend,
    animation_generation: Arc<AtomicU64>,
    pulse_generation: Arc<AtomicU64>,
    current_lighting: Option<Lighting>,
}

impl RgbAdapter {
    pub fn pending(devices: &[Device]) -> Self {
        Self {
            status: RgbStatus {
                reason: if devices.is_empty() {
                    "No Corsair device is connected".into()
                } else {
                    "Negotiating a native Corsair lighting backend…".into()
                },
                ..Default::default()
            },
            backend: RgbBackend::None,
            animation_generation: Arc::new(AtomicU64::new(0)),
            pulse_generation: Arc::new(AtomicU64::new(0)),
            current_lighting: None,
        }
    }

    pub fn probe(devices: &[Device], bragi_control: Option<Arc<Mutex<BragiControl>>>) -> Self {
        if devices.is_empty() {
            return Self::unavailable("No Corsair device is connected".into());
        }

        let native_device = devices
            .iter()
            .find(|device| device.vendor_id == "1b1c" && device.product_id == "2b32");
        let mut native_failure = None;
        if let (Some(device), Some(control)) = (native_device, bragi_control) {
            let probe = control
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|mut control| control.enable_rgb().map_err(|error| error.to_string()));
            match probe {
                Ok(RGB_LED_COUNT) => {
                    return Self {
                        status: RgbStatus {
                            available: true,
                            backend: "Native Bragi USB".into(),
                            device_name: device.name.clone(),
                            reason: String::new(),
                            last_applied: None,
                        },
                        backend: RgbBackend::Bragi { control },
                        animation_generation: Arc::new(AtomicU64::new(0)),
                        pulse_generation: Arc::new(AtomicU64::new(0)),
                        current_lighting: None,
                    };
                }
                Ok(count) => {
                    native_failure = Some(format!(
                        "Native lighting reported {count} LEDs; expected {RGB_LED_COUNT}"
                    ));
                }
                Err(error) => {
                    native_failure = Some(format!("Native lighting probe failed: {error}"))
                }
            }
        }

        let output = Command::new("openrgb")
            .args(["--list-devices", "--noautoconnect", "--loglevel", "2"])
            .output();
        let Ok(output) = output else {
            return Self::unavailable(native_failure.unwrap_or_else(|| {
                "No native lighting channel is available and OpenRGB is not installed".into()
            }));
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let Some((selector, device_name)) = openrgb_ironclaw(&text) else {
            let ids = devices
                .iter()
                .map(|device| format!("{}:{}", device.vendor_id, device.product_id))
                .collect::<Vec<_>>()
                .join(", ");
            let reason = native_failure.unwrap_or_else(|| {
                format!(
                    "OpenRGB does not expose the connected Ironclaw ({ids}). Lighting profiles and previews remain available."
                )
            });
            return Self::unavailable(reason);
        };
        Self {
            status: RgbStatus {
                available: true,
                backend: "OpenRGB CLI".into(),
                device_name,
                reason: String::new(),
                last_applied: None,
            },
            backend: RgbBackend::OpenRgb { selector },
            animation_generation: Arc::new(AtomicU64::new(0)),
            pulse_generation: Arc::new(AtomicU64::new(0)),
            current_lighting: None,
        }
    }

    fn unavailable(reason: String) -> Self {
        Self {
            status: RgbStatus {
                reason,
                ..Default::default()
            },
            backend: RgbBackend::None,
            animation_generation: Arc::new(AtomicU64::new(0)),
            pulse_generation: Arc::new(AtomicU64::new(0)),
            current_lighting: None,
        }
    }

    pub fn apply(&mut self, lighting: &Lighting) -> Result<(), String> {
        if !self.status.available {
            return Err(self.status.reason.clone());
        }
        let animation_token = self.animation_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pulse_generation.fetch_add(1, Ordering::SeqCst);
        self.current_lighting = Some(lighting.clone());

        match &self.backend {
            RgbBackend::None => return Err(self.status.reason.clone()),
            RgbBackend::OpenRgb { selector } => apply_openrgb(selector, lighting)?,
            RgbBackend::Bragi { control } => {
                let first_frame = native_frame(lighting, 0.0, None)?;
                control
                    .lock()
                    .map_err(|error| error.to_string())?
                    .write_rgb(&first_frame)
                    .map_err(|error| error.to_string())?;

                if matches!(
                    lighting.mode.as_str(),
                    "breathing" | "rainbow" | "spectrum" | "wave"
                ) {
                    run_native_animation(
                        control.clone(),
                        lighting.clone(),
                        self.animation_generation.clone(),
                        animation_token,
                    );
                }
            }
        }
        self.status.last_applied = now_seconds();
        Ok(())
    }

    pub fn physical_input(&self) {
        let Some(lighting) = self.current_lighting.clone() else {
            return;
        };
        if lighting.mode != "reactive" {
            return;
        }
        let RgbBackend::Bragi { control } = &self.backend else {
            return;
        };
        let pulse_token = self.pulse_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let animation_token = self.animation_generation.load(Ordering::SeqCst);
        run_native_reactive_pulse(
            control.clone(),
            lighting,
            self.animation_generation.clone(),
            animation_token,
            self.pulse_generation.clone(),
            pulse_token,
        );
    }
}

impl Drop for RgbAdapter {
    fn drop(&mut self) {
        self.animation_generation.fetch_add(1, Ordering::SeqCst);
        self.pulse_generation.fetch_add(1, Ordering::SeqCst);
    }
}

fn now_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn apply_openrgb(selector: &str, lighting: &Lighting) -> Result<(), String> {
    let mode = match lighting.mode.as_str() {
        "off" | "static" | "gradient" => "direct",
        "breathing" => "breathing",
        "rainbow" | "spectrum" => "spectrum cycle",
        "wave" => "rainbow wave",
        "reactive" => "reactive",
        _ => return Err("Unsupported RGB mode".into()),
    };
    let colors = lighting_colors(lighting)?;
    let brightness = lighting.brightness.to_string();
    let speed = lighting.speed.to_string();
    let output = Command::new("openrgb")
        .args([
            "--device",
            selector,
            "--mode",
            mode,
            "--color",
            &colors,
            "--brightness",
            &brightness,
            "--speed",
            &speed,
            "--noautoconnect",
            "--loglevel",
            "2",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let error = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Err(error.trim().to_owned())
    }
}

fn run_native_animation(
    control: Arc<Mutex<BragiControl>>,
    lighting: Lighting,
    generation: Arc<AtomicU64>,
    token: u64,
) {
    thread::spawn(move || {
        let started = Instant::now();
        while generation.load(Ordering::SeqCst) == token {
            let Ok(frame) = native_frame(&lighting, started.elapsed().as_secs_f32(), None) else {
                break;
            };
            let Ok(mut control) = control.lock() else {
                break;
            };
            if control.write_rgb(&frame).is_err() {
                break;
            }
            drop(control);
            thread::sleep(FRAME_INTERVAL);
        }
    });
}

fn run_native_reactive_pulse(
    control: Arc<Mutex<BragiControl>>,
    lighting: Lighting,
    animation_generation: Arc<AtomicU64>,
    animation_token: u64,
    pulse_generation: Arc<AtomicU64>,
    pulse_token: u64,
) {
    thread::spawn(move || {
        for step in 0..=12 {
            if animation_generation.load(Ordering::SeqCst) != animation_token
                || pulse_generation.load(Ordering::SeqCst) != pulse_token
            {
                return;
            }
            let progress = step as f32 / 12.0;
            let intensity = 0.08 + 0.92 * (1.0 - progress).powi(2);
            let Ok(frame) = native_frame(&lighting, 0.0, Some(intensity)) else {
                return;
            };
            let Ok(mut control) = control.lock() else {
                return;
            };
            if control.write_rgb(&frame).is_err() {
                return;
            }
            drop(control);
            thread::sleep(Duration::from_millis(32));
        }
    });
}

fn native_frame(
    lighting: &Lighting,
    elapsed_seconds: f32,
    reactive_intensity: Option<f32>,
) -> Result<[[u8; 3]; RGB_LED_COUNT], String> {
    let primary = parse_color(&lighting.color)?;
    let secondary = parse_color(&lighting.secondary_color)?;
    let period = 5.5 - (f32::from(lighting.speed) / 100.0) * 4.5;
    let phase = (elapsed_seconds / period.max(0.65)).fract();
    let enabled = |zone: &str| lighting.zones.get(zone).copied().unwrap_or(true);

    let mut logo = primary;
    let mut wheel = primary;
    let mut front = primary;
    let mut effect_scale = 1.0;
    match lighting.mode.as_str() {
        "off" => effect_scale = 0.0,
        "static" => {}
        "gradient" => {
            wheel = blend(primary, secondary, 0.5);
            front = secondary;
        }
        "breathing" => {
            effect_scale = 0.12 + 0.88 * ((phase * TAU - TAU / 4.0).sin() + 1.0) / 2.0;
        }
        "rainbow" | "spectrum" => {
            let color = hue_to_rgb(phase * 360.0);
            logo = color;
            wheel = color;
            front = color;
        }
        "wave" => {
            logo = hue_to_rgb(phase * 360.0);
            wheel = hue_to_rgb((phase * 360.0 + 120.0) % 360.0);
            front = hue_to_rgb((phase * 360.0 + 240.0) % 360.0);
        }
        "reactive" => effect_scale = reactive_intensity.unwrap_or(0.08),
        _ => return Err("Unsupported RGB mode".into()),
    }

    let scale = (f32::from(lighting.brightness) / 100.0) * effect_scale;
    logo = if enabled("logo") {
        scale_color(logo, scale)
    } else {
        [0; 3]
    };
    wheel = if enabled("wheel") {
        scale_color(wheel, scale)
    } else {
        [0; 3]
    };
    front = if enabled("front") {
        scale_color(front, scale)
    } else {
        [0; 3]
    };

    // Ironclaw Bragi packets expose six LEDs in this order: logo, wheel,
    // front, and three DPI/front indicator LEDs. The UI presents the latter
    // four as one physical front zone.
    Ok([logo, wheel, front, front, front, front])
}

fn openrgb_ironclaw(output: &str) -> Option<(String, String)> {
    output.lines().find_map(|line| {
        let (index, name) = line.trim().split_once(':')?;
        if index.parse::<usize>().is_ok() && name.to_ascii_lowercase().contains("ironclaw") {
            Some((index.to_owned(), name.trim().to_owned()))
        } else {
            None
        }
    })
}

fn normalized_color(value: &str) -> Result<String, String> {
    let color = value.trim().trim_start_matches('#');
    if color.len() == 6 && color.chars().all(|character| character.is_ascii_hexdigit()) {
        Ok(color.to_ascii_uppercase())
    } else {
        Err(format!("Invalid RGB color: {value}"))
    }
}

fn parse_color(value: &str) -> Result<[u8; 3], String> {
    let color = normalized_color(value)?;
    Ok([
        u8::from_str_radix(&color[0..2], 16).map_err(|error| error.to_string())?,
        u8::from_str_radix(&color[2..4], 16).map_err(|error| error.to_string())?,
        u8::from_str_radix(&color[4..6], 16).map_err(|error| error.to_string())?,
    ])
}

fn scale_color(color: [u8; 3], scale: f32) -> [u8; 3] {
    color.map(|channel| (f32::from(channel) * scale.clamp(0.0, 1.0)).round() as u8)
}

fn blend(left: [u8; 3], right: [u8; 3], amount: f32) -> [u8; 3] {
    std::array::from_fn(|index| {
        (f32::from(left[index]) * (1.0 - amount) + f32::from(right[index]) * amount).round() as u8
    })
}

fn hue_to_rgb(hue: f32) -> [u8; 3] {
    let chroma = 1.0;
    let section = (hue.rem_euclid(360.0)) / 60.0;
    let x = chroma * (1.0 - (section.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match section as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    [red, green, blue].map(|channel| (channel * 255.0).round() as u8)
}

fn lighting_colors(lighting: &Lighting) -> Result<String, String> {
    if lighting.mode == "off" {
        return Ok("000000".into());
    }
    let primary = normalized_color(&lighting.color)?;
    let secondary = normalized_color(&lighting.secondary_color)?;
    let enabled = |zone: &str| lighting.zones.get(zone).copied().unwrap_or(true);
    let color_for = |zone: &str, color: &str| {
        if enabled(zone) {
            color.to_owned()
        } else {
            "000000".into()
        }
    };
    Ok([
        color_for(
            "front",
            if lighting.mode == "gradient" {
                &secondary
            } else {
                &primary
            },
        ),
        color_for("wheel", &secondary),
        color_for("logo", &primary),
    ]
    .join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn test_lighting(mode: &str) -> Lighting {
        Lighting {
            mode: mode.into(),
            color: "#38d9c5".into(),
            secondary_color: "#ff6b35".into(),
            brightness: 100,
            speed: 45,
            zones: BTreeMap::from([
                ("front".into(), true),
                ("wheel".into(), false),
                ("logo".into(), true),
            ]),
        }
    }

    #[test]
    fn finds_numbered_openrgb_ironclaw() {
        let output = "0: Mainboard\n1: Corsair Ironclaw Wireless SE\n  Type: Mouse\n";
        assert_eq!(
            openrgb_ironclaw(output),
            Some(("1".into(), "Corsair Ironclaw Wireless SE".into()))
        );
    }

    #[test]
    fn creates_three_zone_color_payload() {
        let lighting = test_lighting("gradient");
        assert_eq!(lighting_colors(&lighting).unwrap(), "FF6B35,000000,38D9C5");
    }

    #[test]
    fn maps_front_zone_to_four_bragi_leds() {
        let lighting = test_lighting("gradient");
        let frame = native_frame(&lighting, 0.0, None).unwrap();
        assert_eq!(frame[0], [0x38, 0xd9, 0xc5]);
        assert_eq!(frame[1], [0, 0, 0]);
        assert_eq!(frame[2], [0xff, 0x6b, 0x35]);
        assert_eq!(frame[2], frame[3]);
        assert_eq!(frame[3], frame[4]);
        assert_eq!(frame[4], frame[5]);
    }

    #[test]
    fn turns_every_native_led_off() {
        let lighting = test_lighting("off");
        assert_eq!(
            native_frame(&lighting, 0.0, None).unwrap(),
            [[0; 3]; RGB_LED_COUNT]
        );
    }
}
