#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
desktop_dir="$HOME/.local/share/applications"
desktop_file="$desktop_dir/localflow-linux.desktop"
user_desktop_dir="${XDG_DESKTOP_DIR:-$HOME/Desktop}"
user_desktop_file="$user_desktop_dir/LocalFlow.desktop"
icon_path="$repo_root/src-tauri/icons/128x128.png"

mkdir -p "$desktop_dir"
mkdir -p "$user_desktop_dir"
chmod +x "$repo_root/scripts/Start-LocalFlow-linux.sh"

cat >"$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=LocalFlow
Comment=Local-first dictation
Exec=$repo_root/scripts/Start-LocalFlow-linux.sh
Icon=$icon_path
Terminal=false
Categories=Utility;Accessibility;
StartupNotify=true
EOF

chmod +x "$desktop_file"
install -m 755 "$desktop_file" "$user_desktop_file"

if command -v gio >/dev/null 2>&1; then
  gio set "$desktop_file" metadata::trusted true >/dev/null 2>&1 || true
  gio set "$user_desktop_file" metadata::trusted true >/dev/null 2>&1 || true
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi

echo "Installed LocalFlow desktop shortcut:"
echo "$desktop_file"
echo "$user_desktop_file"
