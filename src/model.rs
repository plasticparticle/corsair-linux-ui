use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub vendor_id: String,
    pub product_id: String,
    pub name: String,
    pub serial: String,
    pub transport: String,
    pub connected: bool,
    pub protocol_supported: bool,
    pub input_nodes: Vec<String>,
}

fn read_trimmed(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

pub fn discover_devices_at(usb_root: &Path, input_root: &Path) -> Vec<Device> {
    let mut devices = Vec::new();
    let Ok(entries) = fs::read_dir(usb_root) else {
        return devices;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let vendor = read_trimmed(path.join("idVendor")).to_lowercase();
        if vendor != "1b1c" {
            continue;
        }
        let product = read_trimmed(path.join("idProduct")).to_lowercase();
        let (fallback, transport, supported) = match product.as_str() {
            "1b4c" => ("Ironclaw RGB Wireless", "usb", true),
            "1b66" => ("Ironclaw RGB Wireless receiver", "slipstream", true),
            "1b5d" => ("Ironclaw RGB", "usb", true),
            // Observed on the connected Ironclaw Wireless SE. This PID is not
            // in ckb-next or OpenRGB as of 2026-08, so raw RGB is guarded.
            "2b32" => ("Ironclaw Wireless SE", "usb", false),
            _ => ("Corsair USB device", "usb", false),
        };
        let serial = {
            let value = read_trimmed(path.join("serial"));
            if value.is_empty() {
                "no-serial".into()
            } else {
                value
            }
        };
        let name = {
            let value = read_trimmed(path.join("product"));
            if value.is_empty() {
                fallback.into()
            } else {
                value
            }
        };
        let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let mut input_nodes = Vec::new();
        if let Ok(events) = fs::read_dir(input_root) {
            for event in events.flatten() {
                let event_name = event.file_name().to_string_lossy().into_owned();
                if !event_name.starts_with("event") {
                    continue;
                }
                if fs::canonicalize(event.path().join("device"))
                    .is_ok_and(|device_path| device_path.starts_with(&canonical))
                {
                    input_nodes.push(format!("/dev/input/{event_name}"));
                }
            }
        }
        input_nodes.sort();
        devices.push(Device {
            id: format!("usb-{vendor}-{product}-{serial}"),
            vendor_id: vendor,
            product_id: product,
            name,
            serial,
            transport: transport.into(),
            connected: true,
            protocol_supported: supported,
            input_nodes,
        });
    }
    // Bluetooth HID uses bus type 0x0005 and is represented by the input
    // subsystem rather than /sys/bus/usb. Configuration reports are not
    // assumed to exist, but event remapping remains available.
    if let Ok(events) = fs::read_dir(input_root) {
        let usb_nodes: std::collections::BTreeSet<_> = devices
            .iter()
            .flat_map(|d| d.input_nodes.iter().cloned())
            .collect();
        for event in events.flatten() {
            let name = event.file_name().to_string_lossy().into_owned();
            if !name.starts_with("event") {
                continue;
            }
            let base = event.path().join("device");
            let vendor = read_trimmed(base.join("id/vendor")).to_lowercase();
            let bus = read_trimmed(base.join("id/bustype")).to_lowercase();
            let node = format!("/dev/input/{name}");
            if vendor.trim_start_matches('0') != "1b1c"
                || bus.trim_start_matches('0') != "5"
                || usb_nodes.contains(&node)
            {
                continue;
            }
            let uniq = read_trimmed(base.join("uniq"));
            let product_id = read_trimmed(base.join("id/product")).to_lowercase();
            let device_name = read_trimmed(base.join("name"));
            let key = format!(
                "bluetooth-{}",
                if uniq.is_empty() { &device_name } else { &uniq }
            );
            if let Some(device) = devices.iter_mut().find(|d| d.id == key) {
                device.input_nodes.push(node);
            } else {
                devices.push(Device {
                    id: key,
                    vendor_id: "1b1c".into(),
                    product_id,
                    name: device_name,
                    serial: uniq,
                    transport: "bluetooth".into(),
                    connected: true,
                    protocol_supported: false,
                    input_nodes: vec![node],
                });
            }
        }
    }
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices
}

pub fn discover_devices() -> Vec<Device> {
    discover_devices_at(
        Path::new("/sys/bus/usb/devices"),
        Path::new("/sys/class/input"),
    )
}

pub fn apply_device_defaults(config: &mut Config, devices: &[Device]) -> bool {
    if !devices
        .iter()
        .any(|device| device.vendor_id == "1b1c" && device.product_id == "2b32")
    {
        return false;
    }

    let defaults = [
        ("forward", "BRAGI_BUTTON:4"),
        ("back", "BRAGI_BUTTON:5"),
        ("dpiup", "BRAGI_BUTTON:6"),
        ("dpidn", "BRAGI_BUTTON:7"),
        ("profup", "BRAGI_BUTTON:8"),
        ("profdn", "BRAGI_BUTTON:9"),
        ("option", "BRAGI_BUTTON:10"),
    ];
    let mut changed = false;
    for profile in &mut config.profiles {
        for (button, source) in defaults {
            let replace_legacy = matches!(
                (button, profile.sources.get(button).map(String::as_str)),
                ("forward", Some("EV_KEY:276" | "EV_KEY:277"))
                    | ("back", Some("EV_KEY:275" | "EV_KEY:278"))
                    | ("dpiup", Some("BRAGI_BUTTON:8"))
                    | ("dpidn", Some("BRAGI_BUTTON:9"))
                    | ("profup", Some("BRAGI_BUTTON:6"))
                    | ("profdn", Some("BRAGI_BUTTON:7"))
            );
            if !profile.sources.contains_key(button) || replace_legacy {
                profile.sources.insert(button.into(), source.into());
                changed = true;
            }
        }
    }
    changed
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub action: String,
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lighting {
    pub mode: String,
    pub color: String,
    pub secondary_color: String,
    pub brightness: u8,
    pub speed: u8,
    pub zones: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub accent: String,
    pub dpi_stages: Vec<u32>,
    pub active_dpi: usize,
    pub polling_rate: u32,
    pub lighting: Lighting,
    pub mappings: BTreeMap<String, Mapping>,
    pub sources: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub version: u8,
    pub active_profile: String,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub source: &'static str,
}

pub fn buttons() -> Vec<ButtonDefinition> {
    vec![
        ButtonDefinition {
            id: "mouse1",
            name: "Left click",
            source: "BTN_LEFT",
        },
        ButtonDefinition {
            id: "mouse2",
            name: "Right click",
            source: "BTN_RIGHT",
        },
        ButtonDefinition {
            id: "mouse3",
            name: "Wheel click",
            source: "BTN_MIDDLE",
        },
        ButtonDefinition {
            id: "wheelup",
            name: "Wheel up",
            source: "REL_WHEEL_UP",
        },
        ButtonDefinition {
            id: "wheeldn",
            name: "Wheel down",
            source: "REL_WHEEL_DOWN",
        },
        ButtonDefinition {
            id: "dpiup",
            name: "DPI up",
            source: "LEARN",
        },
        ButtonDefinition {
            id: "dpidn",
            name: "DPI down",
            source: "LEARN",
        },
        ButtonDefinition {
            id: "profup",
            name: "Profile up",
            source: "LEARN",
        },
        ButtonDefinition {
            id: "profdn",
            name: "Profile down",
            source: "LEARN",
        },
        ButtonDefinition {
            id: "forward",
            name: "Forward",
            source: "BTN_FORWARD",
        },
        ButtonDefinition {
            id: "back",
            name: "Back",
            source: "BTN_BACK",
        },
        ButtonDefinition {
            id: "option",
            name: "Option",
            source: "LEARN",
        },
    ]
}

fn mapping(action: &str, value: &str, label: &str) -> Mapping {
    Mapping {
        action: action.into(),
        value: value.into(),
        label: label.into(),
    }
}

impl Default for Config {
    fn default() -> Self {
        let mappings = BTreeMap::from([
            (
                "mouse1".into(),
                mapping("passthrough", "BTN_LEFT", "Left click"),
            ),
            (
                "mouse2".into(),
                mapping("passthrough", "BTN_RIGHT", "Right click"),
            ),
            (
                "mouse3".into(),
                mapping("passthrough", "BTN_MIDDLE", "Wheel click"),
            ),
            (
                "wheelup".into(),
                mapping("passthrough", "REL_WHEEL_UP", "Scroll up"),
            ),
            (
                "wheeldn".into(),
                mapping("passthrough", "REL_WHEEL_DOWN", "Scroll down"),
            ),
            ("dpiup".into(), mapping("dpi", "next", "Next DPI")),
            ("dpidn".into(), mapping("dpi", "previous", "Previous DPI")),
            ("profup".into(), mapping("profile", "next", "Next profile")),
            (
                "profdn".into(),
                mapping("profile", "previous", "Previous profile"),
            ),
            (
                "forward".into(),
                mapping("passthrough", "BTN_FORWARD", "Forward"),
            ),
            ("back".into(), mapping("passthrough", "BTN_BACK", "Back")),
            (
                "option".into(),
                mapping("shortcut", "CTRL+ALT+T", "Open terminal"),
            ),
        ]);
        let sources = BTreeMap::from([
            ("mouse1".into(), "EV_KEY:272".into()),
            ("mouse2".into(), "EV_KEY:273".into()),
            ("mouse3".into(), "EV_KEY:274".into()),
            ("wheelup".into(), "EV_REL:8:1".into()),
            ("wheeldn".into(), "EV_REL:8:-1".into()),
            ("forward".into(), "EV_KEY:277".into()),
            ("back".into(), "EV_KEY:278".into()),
        ]);
        let lighting = Lighting {
            mode: "static".into(),
            color: "#38d9c5".into(),
            secondary_color: "#ff6b35".into(),
            brightness: 72,
            speed: 45,
            zones: BTreeMap::from([
                ("logo".into(), true),
                ("wheel".into(), true),
                ("front".into(), true),
            ]),
        };
        let profile = Profile {
            id: "desktop".into(),
            name: "Desktop".into(),
            accent: "#38d9c5".into(),
            dpi_stages: vec![800, 1600, 3200],
            active_dpi: 1,
            polling_rate: 1000,
            lighting,
            mappings,
            sources,
        };
        Self {
            version: 1,
            active_profile: profile.id.clone(),
            profiles: vec![profile],
        }
    }
}

impl Config {
    pub fn active(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile)
            .unwrap_or(&self.profiles[0])
    }

    pub fn validate_profile(profile: &Profile) -> Result<(), String> {
        if profile.id.is_empty()
            || profile.id.len() > 32
            || !profile
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err("Invalid profile id".into());
        }
        if profile.name.trim().is_empty() || profile.name.len() > 40 {
            return Err("Profile name must be 1–40 characters".into());
        }
        if profile.dpi_stages.is_empty()
            || profile.dpi_stages.len() > 6
            || profile
                .dpi_stages
                .iter()
                .any(|dpi| !(100..=26000).contains(dpi))
        {
            return Err("Profiles need 1–6 DPI stages between 100 and 26000".into());
        }
        if profile.active_dpi >= profile.dpi_stages.len() {
            return Err("Active DPI stage is out of range".into());
        }
        if ![125, 250, 500, 1000, 2000].contains(&profile.polling_rate) {
            return Err("Unsupported polling rate".into());
        }
        if ![
            "static",
            "gradient",
            "breathing",
            "rainbow",
            "spectrum",
            "wave",
            "reactive",
            "off",
        ]
        .contains(&profile.lighting.mode.as_str())
        {
            return Err("Unsupported lighting mode".into());
        }
        if profile.lighting.brightness > 100 || profile.lighting.speed > 100 {
            return Err("Lighting values must be 0–100".into());
        }
        Ok(())
    }
}

pub fn config_path() -> PathBuf {
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(root).join("corsair-control/config.json");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/corsair-control/config.json")
}

pub fn load_config(path: &Path) -> Config {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_config(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_valid() {
        Config::validate_profile(Config::default().active()).unwrap();
    }
    #[test]
    fn rejects_bad_dpi() {
        let mut profile = Config::default().active().clone();
        profile.dpi_stages = vec![50];
        assert!(Config::validate_profile(&profile).is_err());
    }

    #[test]
    fn wireless_se_gets_extended_button_sources() {
        let mut config = Config::default();
        let device = Device {
            id: "usb-1b1c-2b32-test".into(),
            vendor_id: "1b1c".into(),
            product_id: "2b32".into(),
            name: "Ironclaw Wireless SE".into(),
            serial: "test".into(),
            transport: "usb".into(),
            connected: true,
            protocol_supported: false,
            input_nodes: vec![],
        };
        assert!(apply_device_defaults(&mut config, &[device]));
        let sources = &config.active().sources;
        assert_eq!(sources["forward"], "BRAGI_BUTTON:4");
        assert_eq!(sources["option"], "BRAGI_BUTTON:10");
        assert_eq!(sources["dpiup"], "BRAGI_BUTTON:6");
        assert_eq!(sources["profup"], "BRAGI_BUTTON:8");
    }
}
