# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Stage 1 — 构建管理后台（Vite + React + TS）
# ---------------------------------------------------------------------------
FROM node:22-alpine AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# ---------------------------------------------------------------------------
# Stage 2 — 构建 Rust 后端（把前端产物内嵌进二进制）
# ---------------------------------------------------------------------------
# ⚠️ 必须 ≥ Cargo.lock 里依赖声明的最高 rust-version（当前由 uuid 1.28.0 定为 1.89）。
# 改这个版本时请同时更新 Cargo.toml 的 rust-version 与下方 CI 的 msrv job。
FROM rust:1.89-bookworm AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*

# 先复制清单以利用依赖层缓存
COPY Cargo.toml Cargo.lock ./
COPY crates/alkh-domain/Cargo.toml crates/alkh-domain/
COPY crates/alkh-app/Cargo.toml crates/alkh-app/
COPY crates/alkh-infra/Cargo.toml crates/alkh-infra/
COPY crates/alkh-server/Cargo.toml crates/alkh-server/
# 先用桩代码预热依赖编译缓存（源码变动时这一层仍然命中）
RUN mkdir -p crates/alkh-domain/src crates/alkh-app/src crates/alkh-infra/src \
        crates/alkh-server/src crates/alkh-server/web-dist \
    && echo '' > crates/alkh-domain/src/lib.rs \
    && echo '' > crates/alkh-app/src/lib.rs \
    && echo '' > crates/alkh-infra/src/lib.rs \
    && echo 'fn main() {}' > crates/alkh-server/src/main.rs \
    && echo '<!doctype html><title>build placeholder</title>' > crates/alkh-server/web-dist/index.html \
    && cargo build --release --bin alkh \
    && rm -rf crates/alkh-domain/src crates/alkh-app/src crates/alkh-infra/src crates/alkh-server/src

COPY crates ./crates
COPY migrations ./migrations
COPY --from=web /web/dist ./crates/alkh-server/web-dist
RUN touch crates/*/src/lib.rs crates/alkh-server/src/main.rs \
    && cargo build --release --bin alkh

# ---------------------------------------------------------------------------
# Stage 3 — 运行镜像
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /home/alkh alkh \
    && mkdir -p /data && chown -R alkh:alkh /data

COPY --from=builder /src/target/release/alkh /usr/local/bin/alkh

USER alkh
ENV ALKH_DATA_DIR=/data \
    ALKH_HTTP_ADDR=0.0.0.0:8080 \
    ALKH_LOG_LEVEL=info

VOLUME ["/data"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/alkh", "--healthcheck"]

ENTRYPOINT ["/usr/local/bin/alkh"]
