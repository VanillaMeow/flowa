#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$project_dir"

engine=${CONTAINER_ENGINE:-}
if [[ -z $engine ]]; then
    if command -v podman >/dev/null; then
        engine=podman
    else
        engine=docker
    fi
fi

if ! command -v "$engine" >/dev/null; then
    printf '%s\n' 'Install Podman or Docker to build portable executables.' >&2
    exit 1
fi

if [[ $# != 0 ]]; then
    printf 'Usage: %s\n' "$0" >&2
    exit 2
fi

"$engine" build --target builder -t flowa-portable-builder \
    -f tools/portable/Containerfile tools

# Cargo and the media builds use every core unless the caller sets a limit
job_flags=()
if [[ -n ${CARGO_BUILD_JOBS:-} ]]; then
    job_flags=(-e CARGO_BUILD_JOBS="$CARGO_BUILD_JOBS")
fi

user_flags=(--user "$(id -u):$(id -g)")
if [[ ${engine##*/} == podman ]]; then
    user_flags=(--userns=keep-id)
fi

"$engine" run --rm "${user_flags[@]}" "${job_flags[@]}" \
    -e CARGO_HOME=/work/target/portable-cargo \
    -e FLOWA_PORTABLE_CPU="${FLOWA_PORTABLE_CPU:-generic}" \
    -v "$project_dir:/work:Z" flowa-portable-builder
