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
    printf '%s\n' 'Install Podman or Docker to build and test portable executables.' >&2
    exit 1
fi

case ${1:-} in
    build)
        if [[ $# != 1 ]]; then
            printf 'Usage: %s build\n' "$0" >&2
            exit 2
        fi

        "$engine" build --target builder -t flowa-portable-builder \
            -f tools/portable/Containerfile tools

        user_flags=(--user "$(id -u):$(id -g)")
        if [[ ${engine##*/} == podman ]]; then
            user_flags=(--userns=keep-id)
        fi

        "$engine" run --rm "${user_flags[@]}" \
            -e CARGO_HOME=/work/target/portable-cargo \
            -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
            -v "$project_dir:/work:Z" flowa-portable-builder
        ;;
    test)
        if [[ $# != 2 || ! -f $2 ]]; then
            printf 'Usage: %s test BINARY\n' "$0" >&2
            exit 2
        fi

        binary=$(realpath "$2")
        "$engine" build --target runtime-check -t flowa-portable-runtime \
            -f tools/portable/Containerfile tools
        "$engine" run --rm --network none \
            -v "$binary:/flowa:ro,Z" flowa-portable-runtime
        ;;
    *)
        printf 'Usage: %s build|test BINARY\n' "$0" >&2
        exit 2
        ;;
esac
