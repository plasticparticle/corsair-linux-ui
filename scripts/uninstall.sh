#!/usr/bin/env bash
set -euo pipefail

sudo rm -f /usr/local/bin/corsair-control
sudo rm -f /etc/udev/rules.d/70-corsair-control.rules
sudo rm -f /usr/local/share/applications/corsair-control.desktop
for size in 16 32 48 64 128 256; do
  sudo rm -f "/usr/local/share/icons/hicolor/${size}x${size}/apps/corsair-control.png"
done
sudo udevadm control --reload-rules

echo "Corsair Control removed. Profiles remain in ~/.config/corsair-control."
