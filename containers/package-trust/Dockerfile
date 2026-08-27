FROM rust:1.85.1-bookworm@sha256:bf7d87666c4da6eace19e06d21bc4859c6e2a5c97a21ac273b0e082112753cf0

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/tmp \
    CC_aarch64_unknown_linux_musl=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_musl=aarch64-linux-gnu-g++ \
    AR_aarch64_unknown_linux_musl=aarch64-linux-gnu-ar \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu=4:12.2.0-3 \
        g++-aarch64-linux-gnu=4:12.2.0-3 \
        binutils-aarch64-linux-gnu=2.40-2 \
    && rm -rf /var/lib/apt/lists/* \
    && test "$(aarch64-linux-gnu-gcc -dumpmachine)" = aarch64-linux-gnu \
    && printf 'int cross_probe(void) { return 0; }\n' >/tmp/cross_probe.c \
    && aarch64-linux-gnu-gcc -c /tmp/cross_probe.c -o /tmp/cross_probe.o \
    && file /tmp/cross_probe.o

RUN /usr/local/cargo/bin/rustup component add rustfmt clippy \
    && /usr/local/cargo/bin/rustup target add aarch64-unknown-linux-musl

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY schemas ./schemas
COPY fixtures ./fixtures
COPY scripts ./scripts
COPY docs ./docs
RUN cargo build --release --locked -p package-manager \
    && cargo build --release --locked -p package-manager --target aarch64-unknown-linux-musl

CMD ["/src/target/release/package-manager", "demo", "/src/fixtures/packages"]
