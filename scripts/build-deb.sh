#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

for command_name in cargo dpkg dpkg-deb dpkg-shlibdeps install; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 1
  fi
done

cargo_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
release_version="${1:-$cargo_version}"
release_version="${release_version#v}"

if [[ -z "$cargo_version" || "$release_version" != "$cargo_version" ]]; then
  echo "Release version '$release_version' must match Cargo.toml version '$cargo_version'." >&2
  exit 1
fi

if [[ ! "$release_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Unsupported release version: $release_version" >&2
  exit 1
fi

deb_version="${release_version//-/~}"
architecture="$(dpkg --print-architecture)"
binary_path="$project_dir/target/release/corsair-control"
output_dir="$project_dir/dist"
output_path="$output_dir/corsair-control_${deb_version}_${architecture}.deb"
temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/corsair-control-deb.XXXXXXXX")"
package_root="$temporary_root/package"
metadata_root="$temporary_root/metadata"

cleanup() {
  if [[ -n "${temporary_root:-}" && -d "$temporary_root" ]]; then
    rm -rf -- "$temporary_root"
  fi
}
trap cleanup EXIT

cargo build --release --locked

install -Dm755 "$binary_path" "$package_root/usr/bin/corsair-control"
install -Dm644 resources/70-corsair-control.rules \
  "$package_root/usr/lib/udev/rules.d/70-corsair-control.rules"
install -Dm644 resources/corsair-control.desktop \
  "$package_root/usr/share/applications/corsair-control.desktop"
install -Dm644 LICENSE "$package_root/usr/share/doc/corsair-control/copyright"

for size in 16 32 48 64 128 256; do
  install -Dm644 "icons/${size}x${size}.png" \
    "$package_root/usr/share/icons/hicolor/${size}x${size}/apps/corsair-control.png"
done

mkdir -p "$metadata_root/debian"
cat >"$metadata_root/debian/control" <<EOF
Source: corsair-control
Section: utils
Priority: optional
Maintainer: Corsair Control contributors <noreply@corsair-control.dev>
Standards-Version: 4.6.2

Package: corsair-control
Architecture: any
Description: Native Linux control surface for Corsair input devices
EOF

runtime_dependencies="$(
  cd "$metadata_root"
  dpkg-shlibdeps -O "$binary_path" | sed -n 's/^shlibs:Depends=//p'
)"

install -d "$package_root/DEBIAN"
cat >"$package_root/DEBIAN/control" <<EOF
Package: corsair-control
Version: $deb_version
Section: utils
Priority: optional
Architecture: $architecture
Maintainer: Corsair Control contributors <noreply@corsair-control.dev>
Homepage: https://github.com/plasticparticle/corsair-linux-ui
Depends: $runtime_dependencies
Installed-Size: $(du -sk "$package_root/usr" | cut -f1)
Description: Native Linux control surface for Corsair input devices
 Configure Corsair mouse buttons, shortcuts, profiles, DPI stages, and RGB
 lighting from a native, chromeless Linux application.
EOF

cat >"$package_root/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e

if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
  udevadm trigger --subsystem-match=input --subsystem-match=misc --subsystem-match=hidraw || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
EOF
chmod 755 "$package_root/DEBIAN/postinst"

cat >"$package_root/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e

if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
EOF
chmod 755 "$package_root/DEBIAN/postrm"

mkdir -p "$output_dir"
dpkg-deb --root-owner-group --build "$package_root" "$output_path"
echo "$output_path"
