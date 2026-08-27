FROM trimui-brick-pro-cfw-baseline@sha256:4a65ce9d7599318b78d1a8edb2e49cab3d89e7f798de906801ca5b79e70ed72e

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
RUN cargo fmt --check \
    && cargo build --locked --release -p bootstrap-probe -p brick-recovery -p brick-diagnostics -p boot-state -p update-agent -p userspace-supervisor \
    && cargo build --locked --release --target aarch64-unknown-linux-musl -p bootstrap-probe -p brick-recovery -p brick-diagnostics -p boot-state -p update-agent -p userspace-supervisor \
    && cargo clippy --locked --release -p bootstrap-probe -p brick-recovery -p brick-diagnostics -p boot-state -p update-agent -p userspace-supervisor -- -D warnings \
    && { rustc --version; cargo --version; rustup target list --installed; file target/release/bootstrap-probe target/release/brick-recovery target/release/brickpro-diagnostics target/release/boot-state target/release/update-agent target/release/userspace-supervisor target/aarch64-unknown-linux-musl/release/bootstrap-probe target/aarch64-unknown-linux-musl/release/brick-recovery target/aarch64-unknown-linux-musl/release/brickpro-diagnostics target/aarch64-unknown-linux-musl/release/boot-state target/aarch64-unknown-linux-musl/release/update-agent target/aarch64-unknown-linux-musl/release/userspace-supervisor; } \
       > /usr/share/trimui-bootstrap-build-versions.txt
