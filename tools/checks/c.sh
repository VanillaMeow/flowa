#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
build_dir="$project_dir/target/native-video"
compiler=${CLANG:-clang}
analyzer=${CLANG_TIDY:-clang-tidy}

if [[ $# -gt 1 || ($# == 1 && $1 != --configure) ]]; then
    printf 'Usage: %s [--configure]\n' "$0" >&2
    exit 2
fi

required_tools=("$compiler" pkg-config)
if [[ ${1:-} != --configure ]]; then
    required_tools+=("$analyzer")
fi
for tool in "${required_tools[@]}"; do
    if ! command -v "$tool" >/dev/null; then
        printf 'Missing C development tool: %s\n' "$tool" >&2
        exit 1
    fi
done

cd "$project_dir"
dependencies=(libavformat libavcodec libavutil libswresample libplacebo sdl3)
pkg_flags=$(pkg-config --cflags "${dependencies[@]}")
read -r -a include_flags <<< "$pkg_flags"
compile_flags=(
    -std=c11 -D_POSIX_C_SOURCE=200809L -fPIC
    -Wall -Wextra -Werror=implicit-function-declaration -Wno-unused-parameter
)

shopt -s globstar nullglob
sources=("$project_dir"/native/**/*.c)
if [[ ${#sources[@]} == 0 ]]; then
    printf '%s\n' 'No native C sources found.' >&2
    exit 1
fi

mkdir -p "$build_dir"
scratch=$(mktemp -d "$build_dir/c-commands.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT

# Let Clang serialize its actual arguments, including escaped paths and SDK include flags
for index in "${!sources[@]}"; do
    "$compiler" "${compile_flags[@]}" "${include_flags[@]}" \
        -fsyntax-only -MJ "$scratch/$index.json" "${sources[index]}"
done

{
    printf '[\n'
    for index in "${!sources[@]}"; do
        if [[ $index != 0 ]]; then
            printf ',\n'
        fi
        sed 's/,[[:space:]]*$//' "$scratch/$index.json"
    done
    printf '\n]\n'
} > "$scratch/compile_commands.json"
mv "$scratch/compile_commands.json" "$build_dir/compile_commands.json"
printf 'C compilation database: %s/compile_commands.json\n' "$build_dir"

if [[ ${1:-} == --configure ]]; then
    exit 0
fi

"$analyzer" --quiet -p "$build_dir" "${sources[@]}"
