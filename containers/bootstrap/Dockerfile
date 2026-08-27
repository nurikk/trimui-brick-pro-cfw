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
RUN cargo fmt --check \
    && cargo build --locked --release -p bootstrap-probe -p brick-recovery \
    && cargo build --locked --release --target aarch64-unknown-linux-musl -p bootstrap-probe -p brick-recovery \
    && cargo clippy --locked --release -p bootstrap-probe -p brick-recovery -- -D warnings \
    && { rustc --version; cargo --version; rustup target list --installed; file target/release/bootstrap-probe target/release/brick-recovery target/aarch64-unknown-linux-musl/release/bootstrap-probe target/aarch64-unknown-linux-musl/release/brick-recovery; } \
       > /usr/share/trimui-bootstrap-build-versions.txt
