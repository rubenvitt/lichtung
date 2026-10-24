# syntax=docker/dockerfile:1.7
#
# Lichtung server image: lichtung-server only (no web UI).
# Build context is this directory (server/).
#
# Multi-arch strategy (linux/amd64 and linux/arm64):
#
#   The Rust builder stages run on the *build* platform
#   (`--platform=$BUILDPLATFORM`) and cross-compile for the target platform.
#   Compiling Rust under QEMU emulation is 5-20x slower and occasionally
#   crashes (rustc/LLVM under qemu-user), so emulation is only used for the
#   tiny runtime stage (apt-get + useradd). rusqlite bundles SQLite and needs
#   a C compiler for the target, so the builder installs the Debian cross gcc
#   for the target architecture when it differs from the build architecture.
#   Builder and runtime are both Debian bookworm, so the glibc versions match.
#
#   Requirement for the server code: no OpenSSL / native-tls dependencies
#   (use rustls), otherwise cross builds would also need target libssl-dev.
#
# Dependency caching: cargo-chef compiles all external crates in their own
# layer, keyed only on Cargo.toml/Cargo.lock, so source changes rebuild only
# the workspace crates.
#
# Build locally (from server/):
#   docker build -t lichtung-server .
#   docker buildx build --platform linux/amd64,linux/arm64 -t lichtung-server .

ARG RUST_VERSION=1.97
ARG DEBIAN_RELEASE=bookworm

# ---------------------------------------------------------------------------
# Stage 1a: Rust toolchain with cargo-chef (identical for all targets)
# ---------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS chef
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src
# Same settings for `chef cook` and `cargo build`, otherwise the cached
# dependencies would be fingerprinted differently and rebuilt.
ENV CARGO_PROFILE_RELEASE_STRIP=symbols \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar

# ---------------------------------------------------------------------------
# Stage 1b: dependency recipe (only manifests matter for the cache key)
# ---------------------------------------------------------------------------
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

# ---------------------------------------------------------------------------
# Stage 1c: cross toolchain for the target, cached dependencies, binary
# ---------------------------------------------------------------------------
FROM chef AS builder
ARG TARGETARCH
# Map the Docker architecture to the Rust target triple. The native gcc of
# the rust image already provides `<triple>-gcc`; the Debian cross packages
# are only installed when the target architecture differs.
RUN set -eux; \
    case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-gnu;  cross="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) target=aarch64-unknown-linux-gnu; cross="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    if [ "$(dpkg --print-architecture)" != "$TARGETARCH" ]; then \
      apt-get update; \
      apt-get install -y --no-install-recommends $cross; \
      rm -rf /var/lib/apt/lists/*; \
    fi; \
    rustup target add "$target"; \
    echo "$target" > /rust-target

COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --target "$(cat /rust-target)" --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN set -eux; \
    target="$(cat /rust-target)"; \
    cargo build --release --locked --target "$target" --bin lichtung-server; \
    mkdir -p /out; \
    cp "target/$target/release/lichtung-server" /out/

# ---------------------------------------------------------------------------
# Stage 2: runtime
# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS runtime
# ca-certificates: outgoing HTTPS (Pocket-ID, later the calendar providers)
# sqlite3:         integrity checks and manual backups/restores (docs/betrieb.md)
# tini:            PID 1, forwards SIGTERM and reaps zombies
# tzdata:          local time for LICHTUNG_BACKUP_HOUR (TZ, default Europe/Berlin)
# The HEALTHCHECK uses the server's own `lichtung-server healthcheck`, no curl.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates sqlite3 tini tzdata; \
    rm -rf /var/lib/apt/lists/*; \
    groupadd --system --gid 10001 lichtung; \
    useradd --system --uid 10001 --gid lichtung --home-dir /data --no-create-home \
      --shell /usr/sbin/nologin lichtung; \
    mkdir -p /data; \
    chown lichtung:lichtung /data

COPY --from=builder /out/lichtung-server /usr/local/bin/

ENV LICHTUNG_BIND=0.0.0.0:8080 \
    LICHTUNG_DATA_DIR=/data \
    TZ=Europe/Berlin

USER lichtung
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8080

# `lichtung-server healthcheck` reads only LICHTUNG_BIND, so it follows a
# changed port. Traefik only routes to a container once it is `healthy`;
# during the start period the probe runs every 2 s.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --start-interval=2s --retries=3 \
  CMD ["lichtung-server", "healthcheck"]

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["lichtung-server"]
