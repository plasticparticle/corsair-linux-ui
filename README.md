<div align="center">
  <img src="icons/icon.png" width="128" alt="Corsair Control icon: a mint mouse with an orange illuminated logo">

  # Corsair Control Driver & UI for Linux

  **Your mouse. Your shortcuts. Your Linux box.**

  A native, chromeless control room for the Corsair Ironclaw—and a growing collection of other Corsair input devices.

  **Unofficial, unaffiliated third-party project. Not endorsed, sponsored, supported, or approved by Corsair.**
</div>

---

> [!CAUTION]
> ## Experimental pre-release — install at your own risk
>
> Corsair Control `v0.1.0` is an early hardware-driver pre-release. It is **far from production-ready or security-hardened**, has not received an independent security audit, and currently has meaningful testing only on one Ironclaw Wireless SE (`1b1c:2b32`).
>
> The application installs a udev permission rule, reads and grabs raw Linux input devices, creates a virtual input device, and sends reverse-engineered commands to mouse hardware. Bugs could break input until the mouse is reconnected, expose unintended device access, lose mappings, or behave differently on untested firmware and distributions. The release packages are unsigned; SHA-256 checksums provide corruption detection, not publisher authentication.
>
> Inspect the source and packaging scripts before installing. Do not use this on a critical workstation or with hardware you cannot afford to troubleshoot. **You accept all risk by installing or running it.**

Corsair makes a very comfortable mouse. Linux makes a very comfortable operating system. The two just needed someone to introduce them properly.

Corsair Control is a Rust-powered desktop app for remapping mouse buttons, recording shortcuts, managing profiles, inspecting connection modes, and—where the hardware protocol is known—controlling RGB. It runs as a proper borderless Linux application using Tauri and WebKitGTK. No browser tab, no mystery localhost server, no Windows VM lurking under the desk.

The app was built around a real **Corsair Ironclaw Wireless SE**, currently identifying itself as `1b1c:2b32`.

## The good bits

| Feature | Status | What it does |
|---|:---:|---|
| Button remapping | ✅ | Pass through, disable, send a key, media control, shortcut, profile action, or DPI-stage action |
| Shortcut recording | ✅ | Press a combination such as `Ctrl` + `Alt` + `T`; the app stores and replays it safely |
| Input learning | ✅ | Press an unknown physical button and teach the profile its Linux event code |
| Live button feedback | ✅ | Physical presses travel directly from Rust to the mouse diagram and glow orange while held |
| Wireless SE extra buttons | ✅ | Switches `2b32` into software-control mode and reads F, B, O, profile, and DPI controls from its extended HID channel |
| Profiles | ✅ | Keep separate layouts for desktop work, games, editing, or that one app with 47 shortcuts |
| USB + Bluetooth discovery | ✅ | Finds Corsair HID devices and reports their current transport |
| Slipstream discovery | ✅ | Recognizes verified receiver IDs when the dongle is present |
| Tray/panel mode | ✅ | Close without stopping mappings; one click restores the window on the active desktop and monitor |
| RGB effect editor | ✅ | Preview static, gradient, breathing, spectrum, wave, reactive, and off patterns across the three real lighting zones |
| Hardware RGB apply | ✅ | Uses the native six-LED Bragi channel on `2b32`, with OpenRGB as a fallback for supported models |
| Native RGB on `2b32` | ✅ | Static, gradient, breathing, spectrum, wave, reactive, and off effects over USB |
| Native DPI on `2b32` | ✅ | P+ and P− walk the profile ladder without wrapping and write the selected DPI directly to both sensor axes over USB |

## Meet the control room

The interface is split into four stations:

- **Assignments** — click a control on the mouse diagram, identify its physical event, then give it a better job.
- **Lighting** — design and save three-zone color profiles, preview seven animated effects, then apply them when a verified RGB backend is available.
- **Performance** — organize up to six DPI stages; the selected stage is applied directly to the Ironclaw sensor over USB.
- **Connections** — see whether the mouse arrived over USB, Slipstream 2.4 GHz, or Bluetooth.

Mint means “selected for editing.” Orange means “the physical button is being pressed right now.” If an unusual firmware sends an unfamiliar code, the raw event appears on the mouse map so it cannot hide forever.

## A tiny driver tour

```mermaid
flowchart LR
    A[Physical Corsair mouse] -->|evdev events| B[Rust input engine]
    B --> C{Profile mapping}
    C -->|unchanged or remapped| D[Linux uinput device]
    D --> E[Desktop / game / app]
    B -->|live press events| F[Native Tauri UI]
    F -->|saved profile| C
    F -->|negotiated Bragi RGB| G[Ironclaw lighting]
    F -. supported fallback .-> H[OpenRGB]
```

The input engine grabs the Corsair event nodes, processes only the configured controls, and relays everything through a virtual Linux input device. Unassigned events pass through unchanged. If the application exits, the kernel releases the grabs automatically—your mouse does not become a stylish paperweight.

Button mappings stay active while the process is running. Closing the window hides it to the tray; choosing **Quit** from the tray actually stops it.

## Install it

### Download the latest Linux packages

The current pre-release is **v0.1.0 for x86-64 Linux**:

| Distribution | Package | Direct download |
|---|---|---|
| Debian, Ubuntu, Linux Mint | `.deb` (`amd64`) | [corsair-control_0.1.0_amd64.deb](https://github.com/plasticparticle/corsair-linux-ui/releases/download/v0.1.0/corsair-control_0.1.0_amd64.deb) |
| Fedora and RPM-based distributions | `.rpm` (`x86_64`) | [corsair-control-0.1.0-1.x86_64.rpm](https://github.com/plasticparticle/corsair-linux-ui/releases/download/v0.1.0/corsair-control-0.1.0-1.x86_64.rpm) |
| Integrity manifest | SHA-256 | [SHA256SUMS](https://github.com/plasticparticle/corsair-linux-ui/releases/download/v0.1.0/SHA256SUMS) |

[Open the v0.1.0 release page](https://github.com/plasticparticle/corsair-linux-ui/releases/tag/v0.1.0) or [browse all releases](https://github.com/plasticparticle/corsair-linux-ui/releases). Packages are built by GitHub Actions from the tagged source.

### Debian, Ubuntu, or Linux Mint

Download the `.deb` and its checksum manifest from the links above, then verify and install it:

```bash
grep 'corsair-control_0.1.0_amd64.deb$' SHA256SUMS | sha256sum --check
sudo apt install ./corsair-control_0.1.0_amd64.deb
```

### Fedora or another RPM-based distribution

RPM support is new and more experimental than the Debian package. Download both files, verify the package, and install it with DNF:

```bash
grep 'corsair-control-0.1.0-1.x86_64.rpm$' SHA256SUMS | sha256sum --check
sudo dnf install ./corsair-control-0.1.0-1.x86_64.rpm
```

Both packages install the application, desktop entry, icons, and udev permissions in standard system locations. Reconnect the mouse once after the first installation, then start **Corsair Control** from the application menu.

### Build from source

#### 1. Install build dependencies

On Ubuntu 24.04, Linux Mint, and related Debian-based systems:

```bash
sudo apt install build-essential curl file \
  libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

OpenRGB is optional. Install it only when using an older supported Ironclaw revision that does not expose the native `2b32` Bragi backend:

```bash
sudo apt install openrgb
```

#### 2. Build and install

```bash
chmod +x scripts/install.sh scripts/uninstall.sh
./scripts/install.sh
```

The installer builds an optimized Rust binary and adds:

- the application at `/usr/local/bin/corsair-control`;
- Linux input permissions at `/etc/udev/rules.d/70-corsair-control.rules`;
- an application-menu entry;
- crisp versions of the custom icon from 16×16 through 256×256.

Reconnect the mouse once after installation, then open **Corsair Control** from your application menu.

Prefer to look around before installing anything system-wide? Run it from the repository:

```bash
cargo run
```

The interface will open, although the input engine may request permissions until the udev rule is installed.

## Your first remap in 30 seconds

1. Open **Assignments**.
2. Pick a button on the Ironclaw diagram.
3. For an unknown extra button, choose **Identify Input** and press it once.
4. Select an action—or hit **Record** and type a shortcut.
5. Choose **Save to Profile**.
6. Press the physical button and enjoy the orange glow of immediate gratification.

Profiles are written atomically to:

```text
${XDG_CONFIG_HOME:-~/.config}/corsair-control/config.json
```

No arbitrary shell commands are executed by button mappings. That feature was left out on purpose; a mouse button should open your terminal because you mapped a shortcut, not because it quietly became root.

## The tray icon has one job (well, two)

Closing the main window hides it and leaves the Rust input engine running.

- **Left-click the tray icon** to restore, raise, and focus the window on the desktop and monitor you are currently using.
- **Open Corsair Control** in the right-click menu performs the same restore action.
- **Quit** releases the input devices and stops the application.

Clicking the application launcher or taskbar icon while Corsair Control is already running also restores the existing process instead of opening a second copy.

The same mint-and-orange mouse icon is used in the system tray, taskbar, and desktop application menu, with dedicated sizes so it stays sharp instead of becoming a tiny turquoise smudge.

## About the `2b32`-shaped elephant in the room

The Ironclaw Wireless SE connected during development reports:

```text
Vendor:  1b1c
Product: 2b32
Name:    CORSAIR IRONCLAW WIRELESS SE Gaming Mouse
```

That product ID is newer than the Ironclaw definitions currently used by ckb-next and OpenRGB. It does, however, expose the same negotiated Bragi lighting resource family used by maintained Corsair drivers. Corsair Control requires the exact `2b32` command descriptor and a successful lighting-resource open before enabling the known six-LED payload; firmware that supports the optional size query is checked as well.

For `2b32`, today:

- Linux input, all seven extra controls, remapping, shortcuts, live button feedback, and profiles work. The driver restores hardware mode when it exits.
- Six-LED RGB and physical sensor-DPI writes work over USB. Polling-rate writes, firmware operations, and receiver pairing remain capability-locked.
- The centre P+ and P− controls move up and down the DPI ladder and stop at its ends. The side D+ and D− controls send Page Up and Page Down by default.
- The UI lets you design, animate, and save static, gradient, breathing, spectrum, wave, reactive, and off lighting profiles for the logo, wheel, and front grille.
- **Apply to Device** unlocks after the native lighting resource passes negotiation. The Rust lighting engine drives breathing, spectrum, wave, and reactive effects; OpenRGB remains an automatic fallback for compatible older models.

Bluetooth generally exposes standard HID input but may not expose Corsair's configuration channel. Slipstream capabilities depend on the receiver PID and firmware. Switch transports, open **Connections**, and hit **Rescan**.

This caution is intentional. “Did not brick the mouse” is an underrated feature.

## Project map

```text
src/
├── bragi.rs       guarded native Ironclaw USB transport for DPI, RGB, and extended controls
├── input.rs       evdev capture, uinput relay, shortcuts, live press events
├── main.rs        Tauri commands, single-instance focus, window behavior, Linux tray activation
├── model.rs       device discovery, profiles, validation, persistence
└── rgb.rs         guarded OpenRGB adapter

ui/
├── index.html     control-room structure and mouse diagram
├── styles.css     industrial interface styling
├── mouse-diagram.css detailed Ironclaw Wireless SE illustration and full-height profile map
├── lighting-effects.css three-zone preview and animated RGB patterns
├── action-editor.css context-sensitive action controls and shortcut recorder
├── hotspots.css   physical button and title-bar interaction states
└── app.js         profile editor and Rust ↔ UI bridge

resources/         udev rules and desktop launcher
icons/             source SVG and Linux icon sizes
scripts/           installer and uninstaller
```

## Development

The project intentionally keeps the dependency list small: Rust, Tauri, Serde, and direct Linux system calls. The webview frontend is plain HTML, CSS, and JavaScript, so there is no separate Node build pipeline waiting to download half the internet.

Run the checks:

```bash
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
node --check ui/app.js
bash -n scripts/build-deb.sh scripts/build-rpm.sh scripts/install.sh scripts/uninstall.sh
```

Build the same packages produced by GitHub Releases (`rpmbuild` is required for RPM output):

```bash
./scripts/build-deb.sh v0.1.0
./scripts/build-rpm.sh v0.1.0
```

The release tag must match the version in `Cargo.toml`; successful packages land in `dist/`.

Build the optimized binary:

```bash
cargo build --release --locked
```

Protocol work for `1b1c:2b32`, additional Corsair devices, event-code reports, and tasteful icon improvements are all welcome.

## Uninstall

```bash
./scripts/uninstall.sh
```

The uninstaller removes the binary, udev rule, desktop entry, and installed icons. Your profiles are left in `~/.config/corsair-control` so a future reinstall remembers which button launches your calculator.

## License

Corsair Control is available under **GPL-3.0-or-later**.

The OpenRGB adapter invokes OpenRGB as a separate installed application; no OpenRGB protocol source is copied into this repository.

Corsair, Ironclaw, and related product names and marks belong to their respective owners. Their use here identifies compatible hardware and does not imply any affiliation with or endorsement by Corsair.
