# ---- Build stage ----
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

ARG CPU_TARGET=""

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY assets/ assets/

RUN HOST_TRIPLE=$(rustc -vV | awk '/^host:/ {print $2}') && \
    cargo build --release --target "$HOST_TRIPLE" \
        ${CPU_TARGET:+--config "target.'$HOST_TRIPLE'.rustflags=['-C', 'target-cpu=$CPU_TARGET']"} \
    && strip "target/$HOST_TRIPLE/release/fastqc" \
    && cp "target/$HOST_TRIPLE/release/fastqc" /fastqc

# ---- Runtime stage ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    procps \
    zlib1g \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /fastqc /usr/local/bin/fastqc

CMD ["fastqc"]
