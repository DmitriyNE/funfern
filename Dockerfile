FROM rust:1.96-bookworm AS builder

RUN rustup target add wasm32-unknown-unknown \
    && cargo install trunk --version 0.21.14 --locked

WORKDIR /opt/funfern

ARG TARGETARCH

COPY Cargo.toml Cargo.lock Trunk.toml index.html ./
COPY crates ./crates
COPY assets ./assets

RUN --mount=type=cache,id=funfern-wasm-target-${TARGETARCH},target=/opt/funfern/target \
    --mount=type=cache,id=funfern-cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=funfern-trunk-tools-${TARGETARCH},target=/root/.cache/trunk \
    CARGO_NET_RETRY=10 CARGO_HTTP_TIMEOUT=600 NO_COLOR=true trunk build --release

FROM nginx:1.29-alpine

COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /opt/funfern/dist /usr/share/nginx/html

EXPOSE 80

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1/ || exit 1
