FROM rust:1.85.1-bookworm@sha256:bf7d87666c4da6eace19e06d21bc4859c6e2a5c97a21ac273b0e082112753cf0

ARG DOCKER_CACHE_SCOPE
LABEL org.trimui-brick-pro-cfw.cache-scope="$DOCKER_CACHE_SCOPE"

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/tmp

RUN rustup component add rustfmt clippy \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        libsdl2-dev=2.26.5+dfsg-1 \
        libsdl2-ttf-dev=2.20.1+dfsg-2 \
        xvfb=2:21.1.7-3+deb12u13 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY config ./config
COPY fixtures ./fixtures
COPY schemas ./schemas
COPY themes ./themes
COPY sim ./sim
RUN cargo build --release --locked \
    && cargo build --release --locked -p package-manager \
    && install -Dm755 target/release/sim-launcher /usr/local/bin/sim-launcher \
    && install -Dm755 target/release/sim-control /usr/local/bin/sim-control \
    && install -Dm755 target/release/package-manager /usr/local/bin/package-manager \
    && { rustc --version; cargo --version; rustup component list --installed; dpkg-query -W -f='${Package}=${Version}\n' libsdl2-dev libsdl2-ttf-dev xvfb; } \
       > /usr/share/trimui-sim-versions.txt

COPY scripts/sim-entrypoint.sh /usr/local/bin/sim-entrypoint
RUN chmod 755 /usr/local/bin/sim-entrypoint \
    && groupadd --gid 10001 sim \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin sim

USER 10001:10001
ENTRYPOINT ["/usr/local/bin/sim-entrypoint"]
