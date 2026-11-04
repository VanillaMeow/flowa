#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 ]]; then
    printf 'Usage: %s BINARY\n' "$0" >&2
    exit 1
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
binary=$(realpath "$1")
work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT

expect_rejection() {
    local message=$1 target=$2 baseline=${3:-2.39}

    if "$script_dir/check-portable.sh" "$target" "$baseline" > "$work_dir/check.log" 2>&1; then
        printf 'Compatibility check unexpectedly accepted: %s\n' "$message" >&2
        exit 1
    fi

    if ! grep -Fq "$message" "$work_dir/check.log"; then
        cat "$work_dir/check.log" >&2
        exit 1
    fi
}

"$script_dir/check-portable.sh" "$binary" 2.39
expect_rejection 'exceeding the' "$binary" 2.1

# Each mutation uses its own copy of the executable
cp "$binary" "$work_dir/shared-codec"
patchelf --add-needed libavcodec.so.63 "$work_dir/shared-codec"
expect_rejection 'Unexpected shared dependency' "$work_dir/shared-codec"

cp "$binary" "$work_dir/absolute-path"
patchelf --set-rpath /build-machine/lib "$work_dir/absolute-path"
expect_rejection 'must not contain a library search path' "$work_dir/absolute-path"

printf '%s\n' 'Portable dependency, search-path, and glibc rejection checks passed'
