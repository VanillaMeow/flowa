#!/bin/sh
set -eu
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
rm -f -- "$HOME/.local/bin/flowa" "$data_dir/applications/flowa.desktop" "$data_dir/icons/hicolor/scalable/apps/flowa.svg"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications"
fi
printf '%s\n' 'Removed flowa. Preferences were preserved.'
