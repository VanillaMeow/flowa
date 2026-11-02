FROM docker.io/library/ubuntu:24.04

ARG RUST_VERSION=1.99.0
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl pkg-config cmake meson ninja-build nasm \
        xz-utils binutils \
    && rm -rf /var/lib/apt/lists/*
RUN curl --fail --location --retry 3 https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain "$RUST_VERSION" \
    && rm /tmp/rustup-init.sh

WORKDIR /work
CMD [ \
    "bash", "-c", "./packaging/linux/build-portable.sh && \
    ./packaging/linux/check-portable.sh \"${CARGO_TARGET_DIR:-target}/x86_64-unknown-linux-gnu/portable/flowa\" 2.39" \
]
