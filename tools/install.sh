#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_dir="$HOME/.local/bin"

case ${1:-} in
    install)
        binary=${2:-"${CARGO_TARGET_DIR:-$project_dir/target}/release/flowa"}
        if [ ! -x "$binary" ]; then
            printf 'No build found at %s. Build it first with just.\n' "$binary" >&2
            exit 1
        fi

        install -Dm755 "$binary" "$bin_dir/flowa"
        rm -f -- "$bin_dir/libflowa-video.so"
        install -Dm644 "$project_dir/assets/flowa.svg" "$data_dir/icons/hicolor/scalable/apps/flowa.svg"
        install -Dm644 "$project_dir/packaging/linux/flowa.desktop" "$data_dir/applications/flowa.desktop"
        printf 'Installed flowa to %s\n' "$bin_dir/flowa"
        ;;
    uninstall)
        rm -f -- "$bin_dir/flowa" "$bin_dir/libflowa-video.so" \
            "$data_dir/applications/flowa.desktop" \
            "$data_dir/icons/hicolor/scalable/apps/flowa.svg"
        printf '%s\n' 'Removed flowa. Preferences were preserved.'
        ;;
    *)
        printf 'Usage: %s install [BINARY] | uninstall\n' "$0" >&2
        exit 2
        ;;
esac

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications"
fi
