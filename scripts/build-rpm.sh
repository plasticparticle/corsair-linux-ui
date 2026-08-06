#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

for command_name in cargo rpmbuild install; do
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

if [[ ! "$release_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "RPM releases currently require a stable semantic version: $release_version" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64) rpm_arch="x86_64" ;;
  aarch64) rpm_arch="aarch64" ;;
  *)
    echo "Unsupported RPM architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

binary_path="$project_dir/target/release/corsair-control"
output_dir="$project_dir/dist"
output_path="$output_dir/corsair-control-${release_version}-1.${rpm_arch}.rpm"
temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/corsair-control-rpm.XXXXXXXX")"
rpm_root="$temporary_root/rpmbuild"
spec_path="$rpm_root/SPECS/corsair-control.spec"

cleanup() {
  if [[ -n "${temporary_root:-}" && -d "$temporary_root" ]]; then
    rm -rf -- "$temporary_root"
  fi
}
trap cleanup EXIT

cargo build --release --locked

mkdir -p "$rpm_root"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS,rpmdb,tmp}
cat >"$spec_path" <<EOF
Name:           corsair-control
Version:        $release_version
Release:        1
Summary:        Native Linux control surface for Corsair input devices
License:        GPL-3.0-or-later
URL:            https://github.com/plasticparticle/corsair-linux-ui

%global debug_package %{nil}

%description
Configure Corsair mouse buttons, shortcuts, profiles, DPI stages, and RGB
lighting from a native, chromeless Linux application.

%prep

%build

%install
install -Dm755 %{project_dir}/target/release/corsair-control \
  %{buildroot}%{_bindir}/corsair-control
install -Dm644 %{project_dir}/resources/70-corsair-control.rules \
  %{buildroot}%{_prefix}/lib/udev/rules.d/70-corsair-control.rules
install -Dm644 %{project_dir}/resources/corsair-control.desktop \
  %{buildroot}%{_datadir}/applications/corsair-control.desktop
install -Dm644 %{project_dir}/LICENSE \
  %{buildroot}%{_datadir}/licenses/corsair-control/LICENSE
for size in 16 32 48 64 128 256; do
  install -Dm644 %{project_dir}/icons/\${size}x\${size}.png \
    %{buildroot}%{_datadir}/icons/hicolor/\${size}x\${size}/apps/corsair-control.png
done

%post
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
  udevadm trigger --subsystem-match=input --subsystem-match=misc --subsystem-match=hidraw || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database %{_datadir}/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f %{_datadir}/icons/hicolor >/dev/null 2>&1 || true
fi

%postun
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database %{_datadir}/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f %{_datadir}/icons/hicolor >/dev/null 2>&1 || true
fi

%files
%{_bindir}/corsair-control
%{_prefix}/lib/udev/rules.d/70-corsair-control.rules
%{_datadir}/applications/corsair-control.desktop
%{_datadir}/icons/hicolor/*/apps/corsair-control.png
%license %{_datadir}/licenses/corsair-control/LICENSE
EOF

rpmbuild -bb "$spec_path" \
  --define "_topdir $rpm_root" \
  --define "_dbpath $rpm_root/rpmdb" \
  --define "_tmppath $rpm_root/tmp" \
  --define "project_dir $project_dir" \
  --define "_build_id_links none"

built_path="$(find "$rpm_root/RPMS" -type f -name '*.rpm' -print -quit)"
if [[ -z "$built_path" ]]; then
  echo "rpmbuild did not produce an RPM package." >&2
  exit 1
fi

mkdir -p "$output_dir"
cp "$built_path" "$output_path"
echo "$output_path"
