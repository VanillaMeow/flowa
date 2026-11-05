#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

if [[ $# -lt 1 || $# -gt 2 ]]; then
    printf 'Usage: %s BINARY [MAX_GLIBC_VERSION]\n' "$0" >&2
    exit 1
fi

binary=$1
max_glibc=${2:-}
dynamic_section=$(readelf --wide --dynamic "$binary")
versions=$(readelf --wide --version-info "$binary")

if [[ $dynamic_section == *'(RPATH)'* || $dynamic_section == *'(RUNPATH)'* ]]; then
    printf '%s\n' 'Portable executable must not contain a library search path.' >&2
    exit 1
fi

dependencies=$(sed -n 's/.*(NEEDED).*\[\([^]]*\)\].*/\1/p' <<< "$dynamic_section")
if [[ -z $dependencies ]]; then
    printf '%s\n' 'Expected a GNU/Linux executable with dynamic glibc linkage.' >&2
    exit 1
fi

while IFS= read -r library; do
    case "$library" in
        libc.so.6|libm.so.6|libgcc_s.so.1|libpthread.so.0|libdl.so.2|librt.so.1|libutil.so.1|libresolv.so.2|ld-linux-x86-64.so.2|libvulkan.so.1) ;;
        *)
            printf 'Unexpected shared dependency: %s\n' "$library" >&2
            exit 1
            ;;
    esac
done <<< "$dependencies"

if [[ $versions == *GLIBCXX_* || $versions == *CXXABI_* ]]; then
    printf '%s\n' 'The C++ runtime must be linked statically.' >&2
    exit 1
fi

required_glibc=$(sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' <<< "$versions" | sort -Vu | tail -n 1)
if [[ -z $required_glibc ]]; then
    printf '%s\n' 'Could not determine the required glibc version.' >&2
    exit 1
fi

if [[ -n $max_glibc ]]; then
    newest=$(printf '%s\n%s\n' "$required_glibc" "$max_glibc" | sort -V | tail -n 1)

    if [[ $newest != "$max_glibc" ]]; then
        printf 'Executable requires glibc %s, exceeding the %s baseline.\n' "$required_glibc" "$max_glibc" >&2
        exit 1
    fi
fi

printf 'Required glibc: %s\nShared dependencies:\n%s\n' "$required_glibc" "$dependencies"
