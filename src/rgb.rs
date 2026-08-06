use crate::model::{Device, Lighting};
use serde::Serialize;
use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbStatus {
    pub available: bool,
    pub backend: String,
    pub device_name: String,
    pub reason: String,
    pub last_applied: Option<u64>,
}

pub struct RgbAdapter {
    pub status: RgbStatus,
    device_selector: String,
}

impl RgbAdapter {
    pub fn pending(devices: &[Device]) -> Self {
        Self {
            status: RgbStatus {
                reason: if devices.is_empty() {
                    "No Corsair device is connected".into()
                } else {
                    "Checking OpenRGB for a compatible Ironclaw controller…".into()
                },
                ..Default::default()
            },
            device_selector: String::new(),
        }
    }

    pub fn probe(devices: &[Device]) -> Self {
        if devices.is_empty() {
            return Self {
                status: RgbStatus {
                    reason: "No Corsair device is connected".into(),
                    ..Default::default()
                },
                device_selector: String::new(),
            };
        }
        let output = Command::new("openrgb")
            .args(["--list-devices", "--noautoconnect", "--loglevel", "2"])
            .output();
        let Ok(output) = output else {
            return Self {
                status: RgbStatus {
                    reason: "OpenRGB is not installed".into(),
                    ..Default::default()
                },
                device_selector: String::new(),
            };
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
            return Self {
                status: RgbStatus {
                    reason: format!(
                        "OpenRGB is installed, but does not expose the connected Ironclaw ({ids}). Lighting profiles and previews remain available."
                    ),
                    ..Default::default()
                },
                device_selector: String::new(),
            };
        };
        Self {
            status: RgbStatus {
                available: true,
                backend: "OpenRGB CLI".into(),
                device_name,
                reason: String::new(),
                last_applied: None,
            },
            device_selector: selector,
        }
    }

    pub fn apply(&mut self, lighting: &Lighting) -> Result<(), String> {
        if !self.status.available {
            return Err(self.status.reason.clone());
        }
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
                &self.device_selector,
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
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            let error = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Err(error.trim().to_owned());
        }
        self.status.last_applied = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs());
        Ok(())
    }
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
        let lighting = Lighting {
            mode: "gradient".into(),
            color: "#38d9c5".into(),
            secondary_color: "#ff6b35".into(),
            brightness: 72,
            speed: 45,
            zones: BTreeMap::from([
                ("front".into(), true),
                ("wheel".into(), false),
                ("logo".into(), true),
            ]),
        };
        assert_eq!(lighting_colors(&lighting).unwrap(), "FF6B35,000000,38D9C5");
    }
}
