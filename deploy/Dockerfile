# Build context is the parent directory (the repository root), so paths below
# are relative to that. Build from deploy/ with: docker compose build

# ---- Stage 1: frontend (React) -------------------------------------------
FROM node:22-alpine AS frontend
WORKDIR /src/frontend
# Manifests first so the npm layer is cached until dependencies change.
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ---- Stage 2: backend (Rust) ---------------------------------------------
FROM rust:1.97-alpine AS backend
RUN apk add --no-cache musl-dev gcc
WORKDIR /src/backend

# Dependency cache: build a stub main so crate dependencies compile and are
# cached in their own layer. The stub and its artifacts are then removed.
COPY backend/Cargo.toml backend/Cargo.lock* ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && cargo build --release \
 && rm -rf src target/release/dnsui target/release/deps/dnsui-*

# Real source. Touch main.rs so cargo sees it as newer than the stub build.
COPY backend/ ./
RUN touch src/main.rs && cargo build --release

# ---- Stage 3: runtime -----------------------------------------------------
FROM alpine:3.20

# libcap provides setcap, used below. It is removed again after use.
RUN apk add --no-cache ca-certificates libcap su-exec \
 && addgroup -S -g 10001 dnsui \
 && adduser -S -D -H -u 10001 -G dnsui -s /sbin/nologin dnsui

COPY --from=backend /src/backend/target/release/dnsui /usr/local/bin/dnsui

# The process runs as a non-root user, so Linux will not grant it
# CAP_NET_BIND_SERVICE on its own. File capabilities on the binary give it
# the capability at exec time, which lets it bind port 53. The compose file
# adds NET_BIND_SERVICE to the container bounding set so this works.
RUN setcap cap_net_bind_service=+ep /usr/local/bin/dnsui \
 && apk del libcap

COPY --from=frontend /src/frontend/dist /app/static
COPY deploy/config.example.yaml /app/defaults/config.example.yaml
RUN mkdir -p /app/data /app/config && chown -R dnsui:dnsui /app/data
ENV DNSUI_SEED_CONFIG=/app/defaults/config.example.yaml

WORKDIR /app

EXPOSE 53/udp 53/tcp 8080/tcp

# The entrypoint starts as root only to fix ownership of the mounted config
# directory, then drops to the dnsui user with su-exec. The server itself never
# runs as root.
COPY deploy/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 0755 /usr/local/bin/entrypoint.sh

# ENTRYPOINT lets `docker run dnsui hash-password` call the subcommand.
# The default arguments below start the server.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["serve"]
