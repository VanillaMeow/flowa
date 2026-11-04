FROM docker.io/library/ubuntu:24.04 AS runtime-check

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libvulkan1 mesa-vulkan-drivers xvfb xauth python3-pil \
    && rm -rf /var/lib/apt/lists/*

RUN apt-get update && apt-get install -y --no-install-recommends \
        libx11-6 libxrandr2 libxi6 libxcursor1 libxinerama1 libxkbcommon-x11-0 \
    && rm -rf /var/lib/apt/lists/*

FROM docker.io/library/ubuntu:24.04 AS builder

ARG RUST_VERSION=1.99.0
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    FLOWA_MAX_GLIBC=2.39

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl pkg-config cmake meson ninja-build nasm \
        xz-utils binutils patch python3-jinja2 hwdata \
        libass-dev libvulkan-dev libffmpeg-nvenc-dev \
        libasound2-dev libpulse-dev libpipewire-0.3-dev \
    && rm -rf /var/lib/apt/lists/*
RUN curl --fail --location --retry 3 https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain "$RUST_VERSION" \
    && rm /tmp/rustup-init.sh

WORKDIR /work
CMD ["bash", "-c", "./packaging/linux/build-portable.sh"]
