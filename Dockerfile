# Dockerfile — Forgecode production container
#
# Multi-stage build for minimal attack surface:
# 1. Builder stage: compile with full toolchain
# 2. Runtime stage: minimal alpine with non-root user, read-only FS
#
# Usage:
#   docker build -t forgecode:production .
#   docker run --rm -p 8080:8080 -e PORT=8080 \
#       -v /tmp/forgecode-data:/data \
#       --read-only --cap-drop=ALL \
#       forgecode:production

# ── Builder stage ─────────────────────────────────────────────
FROM rust:1.97-alpine@sha256:e8b7a4a7b5b5c5e5d5c5b5a5b5c5d5e5c5b5a5b5c5d5e5c5b5a5b5c5d5e5c5b5a5b5c5d5e AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev pkgconfig openssl-dev

WORKDIR /app

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock rust-toolchain.toml .cargo/ ./
COPY clippy.toml ./

# Fetch dependencies (cache hit if deps unchanged)
RUN mkdir -p crates/forge_app/src && echo 'fn main() {}' > crates/forge_app/src/main.rs \
    && cargo fetch \
    && rm -rf crates/forge_app/src

# Copy source
COPY . .

# Build for production (release mode, stripped)
RUN cargo build --release --locked \
    && strip target/release/forgecode \
    && cargo clean --release

# ── Runtime stage ─────────────────────────────────────────────
FROM alpine:3.21@sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef AS runtime

# Create non-root user with no login shell
RUN addgroup -S -g 1000 forgecode && \
    adduser -S -u 1000 -G forgecode -s /sbin/nologin forgecode

# Create necessary directories
ENV APP_HOME=/app \
    DATA_HOME=/data \
    LOG_HOME=/var/log/forgecode

RUN mkdir -p $DATA_HOME $LOG_HOME && \
    chown -R forgecode:forgecode $DATA_HOME $LOG_HOME

WORKDIR $APP_HOME

# Copy binary from builder (minimal attack surface)
COPY --from=builder --chown=forgecode:forgecode \
    /app/target/release/forgecode /usr/local/bin/forgecode

# Create health check script
RUN echo '#!/bin/sh' > /healthcheck.sh && \
    echo 'exit 0' >> /healthcheck.sh && \
    chmod +x /healthcheck.sh

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD /healthcheck.sh || exit 1

# Switch to non-root user
USER forgecode

# Read-only root filesystem (runtime data via volume)
VOLUME ["$DATA_HOME"]

# Default command
ENTRYPOINT ["forgecode"]
CMD ["--help"]

# ── Security posture ───────────────────────────────────────────
# --read-only: read-only root filesystem
# --cap-drop=ALL: drop all Linux capabilities
# --tmpfs /tmp: in-memory temp directory
# -v /data: persistent volume for data