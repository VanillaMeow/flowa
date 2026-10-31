#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_dir="$HOME/.local/bin"
if [ ! -x "$project_dir/target/release/flowa" ]; then
    printf '%s\n' 'Build first with cargo build --release.' >&2
    exit 1
fi
install -Dm755 "$project_dir/target/release/flowa" "$bin_dir/flowa"
install -Dm644 "$project_dir/assets/flowa.svg" "$data_dir/icons/hicolor/scalable/apps/flowa.svg"
install -Dm644 "$project_dir/packaging/linux/flowa.desktop" "$data_dir/applications/flowa.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications"
fi
printf 'Installed flowa to %s\n' "$bin_dir/flowa"
