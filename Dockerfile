# syntax=docker/dockerfile:1
# Meta Leads: Rust backend with the TypeScript PWA embedded.
# Built natively on the target architecture (an arm64 runner for an arm64 Oracle VM).

FROM node:22-bookworm-slim AS frontend
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# cargo-chef compiles only the dependencies in its own layer, reused until the manifests change.
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /src

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY backend backend
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
# build.rs would run npm; the frontend is built in the stage above.
ENV SKIP_FRONTEND_BUILD=1
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY backend backend
COPY --from=frontend /src/frontend/dist frontend/dist
RUN cargo build --release --locked -p meta-leads-backend

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/meta-leads-backend /usr/local/bin/

# The JSON store lives in /data, a Docker volume.
RUN groupadd --gid 10001 app \
 && useradd --uid 10001 --gid 10001 --home-dir /data --create-home app
ENV DATA_FILE=/data/db.json BIND_ADDR=0.0.0.0:3000
USER app
VOLUME /data
WORKDIR /app
EXPOSE 3000
CMD ["meta-leads-backend"]
