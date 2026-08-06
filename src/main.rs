mod bragi;
mod input;
mod model;
mod rgb;

use input::{DriverStatus, InputDriver};
use model::{ButtonDefinition, Config, Device, Profile};
use rgb::{RgbAdapter, RgbStatus};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
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

fn main() {
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
        .manage(state)
        .setup(move |app| {
            let open = MenuItem::with_id(app, "open", "Open Corsair Control", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("application icon is missing")?;
            TrayIconBuilder::with_id("corsair-control")
                .icon(icon)
                .tooltip("Corsair Control")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

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
            window_close
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Corsair Control");
}
