FROM rust:1.98.0-bookworm@sha256:4e4a7e7939c17991ab35f2b8c2e67593980f771d28f6b1254b1850f860fd0c7f

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
