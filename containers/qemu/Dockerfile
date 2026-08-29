FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867

ARG DOCKER_CACHE_SCOPE
LABEL org.trimui.brick-pro.qemu.base-image="debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867" \
      org.trimui-brick-pro-cfw.cache-scope="$DOCKER_CACHE_SCOPE"

RUN dpkg --add-architecture arm64 \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        qemu-user-static=1:7.2+dfsg-7+deb12u18+b3 \
        busybox-static:arm64=1:1.35.0-4+deb12u1+b1 \
        binutils=2.40-2 \
        coreutils=9.1-1 \
        file=1:5.44-3 \
        jq=1.6-2.1+deb12u2 \
        tar=1.34+dfsg-1.2+deb12u1 \
    && rm -rf /var/lib/apt/lists/* \
    && qemu-aarch64-static --version | head -1 > /usr/share/trimui-qemu-version.txt \
    && qemu-aarch64-static /usr/bin/busybox | head -1 >> /usr/share/trimui-qemu-version.txt

ENTRYPOINT ["/bin/sh"]
