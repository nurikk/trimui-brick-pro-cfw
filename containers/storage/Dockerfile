FROM rust:1.85.1-bookworm@sha256:bf7d87666c4da6eace19e06d21bc4859c6e2a5c97a21ac273b0e082112753cf0

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/tmp

RUN rustup component add rustfmt clippy \
    && rustup target add aarch64-unknown-linux-musl

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
RUN cargo build --release --locked -p storage-layout \
    && cargo build --release --locked -p storage-layout --target aarch64-unknown-linux-musl \
    && { rustc --version; cargo --version; rustup target list --installed; file target/release/storage-layout target/aarch64-unknown-linux-musl/release/storage-layout; } \
       > /usr/share/trimui-storage-build-versions.txt
