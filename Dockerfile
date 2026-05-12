# Stage 1: Build frontend
FROM node:22-alpine AS frontend-builder

WORKDIR /app/web
COPY web/package.json web/package-lock.json* ./
RUN npm ci
COPY web/ ./
RUN npm run build

# Stage 2: Build Rust server
FROM rust:1.85-slim AS server-builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies
COPY server/Cargo.toml server/Cargo.lock* ./server/
RUN mkdir -p server/src && echo 'fn main() {}' > server/src/main.rs
RUN cd server && cargo build --release 2>/dev/null || true
RUN rm server/src/main.rs

# Build actual server
COPY server/src ./server/src
RUN cd server && cargo build --release

# Stage 3: Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=server-builder /app/server/target/release/wyrmhole-server /app/wyrmhole-server
COPY --from=frontend-builder /app/web/dist /app/dist

ENV BIND_ADDR=0.0.0.0:8080
ENV DATA_DIR=/data
ENV STATIC_DIR=/app/dist

EXPOSE 8080

VOLUME ["/data"]

CMD ["/app/wyrmhole-server"]
