#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

cargo build --release --locked
sudo install -Dm755 target/release/corsair-control /usr/local/bin/corsair-control
sudo install -Dm644 resources/70-corsair-control.rules /etc/udev/rules.d/70-corsair-control.rules
sudo install -Dm644 resources/corsair-control.desktop /usr/local/share/applications/corsair-control.desktop
for size in 16 32 48 64 128 256; do
  sudo install -Dm644 "icons/${size}x${size}.png" "/usr/local/share/icons/hicolor/${size}x${size}/apps/corsair-control.png"
done
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input --subsystem-match=misc --subsystem-match=hidraw
if command -v update-desktop-database >/dev/null 2>&1; then
  sudo update-desktop-database /usr/local/share/applications
fi
if command -v xdg-desktop-menu >/dev/null 2>&1; then
  xdg-desktop-menu forceupdate --mode user || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  sudo gtk-update-icon-cache -f /usr/local/share/icons/hicolor >/dev/null 2>&1 || true
fi

echo "Corsair Control installed with its desktop and tray icon. Reconnect the mouse once, then launch it from your applications menu."
