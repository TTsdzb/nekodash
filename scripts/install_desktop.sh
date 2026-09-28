#!/bin/sh
# Run from the extracted Linux archive to install for the current user.
set -eu
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
install -Dm755 "$package_dir/nekodash" "$HOME/.local/bin/nekodash"
install -Dm644 "$package_dir/nekodash.png" "$data_dir/icons/hicolor/256x256/apps/io.github.nekodash.panel.png"
install -Dm644 "$package_dir/io.github.nekodash.panel.desktop" "$data_dir/applications/io.github.nekodash.panel.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications"
fi
printf '%s\n' 'NekoDash installed. Add ~/.local/bin to PATH if it is not already present.'
