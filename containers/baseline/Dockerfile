FROM rust:1.85.1-bookworm@sha256:bf7d87666c4da6eace19e06d21bc4859c6e2a5c97a21ac273b0e082112753cf0

ARG BUILD_DEFINITION_SHA256
LABEL org.trimui.brick-pro-cfw.base-image="rust:1.85.1-bookworm@sha256:bf7d87666c4da6eace19e06d21bc4859c6e2a5c97a21ac273b0e082112753cf0" \
      org.trimui.brick-pro-cfw.build-definition-sha256="$BUILD_DEFINITION_SHA256"

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/tmp \
    CC_aarch64_unknown_linux_musl=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_musl=aarch64-linux-gnu-g++ \
    AR_aarch64_unknown_linux_musl=aarch64-linux-gnu-ar \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld

RUN rustup component add rustfmt clippy \
    && rustup target add aarch64-unknown-linux-musl \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        busybox-static=1:1.35.0-4+deb12u1+b1 \
        gcc-aarch64-linux-gnu=4:12.2.0-3 \
        g++-aarch64-linux-gnu=4:12.2.0-3 \
        binutils-aarch64-linux-gnu=2.40-2 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /source-cache
COPY Cargo.toml Cargo.lock .cargo/ ./
COPY crates ./crates
RUN CARGO_HOME=/opt/cargo-home cargo fetch --locked

RUN { \
      printf '%s\n' 'rustc:'; rustc --version; \
      printf '%s\n' 'cargo:'; cargo --version; \
      printf '%s\n' 'target:'; rustup target list --installed; \
      printf '%s\n' 'components:'; rustup component list --installed; \
      printf '%s\n' 'busybox:'; busybox | head -1; \
    } > /usr/share/trimui-baseline-toolchain.txt

WORKDIR /src
ENTRYPOINT ["/bin/sh"]
