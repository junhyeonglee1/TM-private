FROM rust:1.97-bookworm AS builder

WORKDIR /build
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        build-essential \
        curl \
        file \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        libxdo-dev \
        wget \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY assets ./assets
COPY docs ./docs
COPY src-tauri ./src-tauri
RUN cargo fmt --all -- --check \
    && cargo test --locked --workspace \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo build --locked --release -p tm-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates gosu jq restic sqlite3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home tm

COPY --from=builder /build/target/release/tm-server /usr/local/bin/tm-server
COPY scripts/railway-entrypoint.sh /usr/local/bin/railway-entrypoint
COPY scripts/railway-backup-once.sh /usr/local/bin/railway-backup-once
COPY scripts/railway-backup-loop.sh /usr/local/bin/railway-backup-loop
COPY scripts/railway-backup-restore-drill.sh /usr/local/bin/railway-backup-restore-drill
RUN chmod 0755 \
    /usr/local/bin/tm-server \
    /usr/local/bin/railway-entrypoint \
    /usr/local/bin/railway-backup-once \
    /usr/local/bin/railway-backup-loop \
    /usr/local/bin/railway-backup-restore-drill
RUN sh -n \
    /usr/local/bin/railway-entrypoint \
    /usr/local/bin/railway-backup-once \
    /usr/local/bin/railway-backup-loop \
    /usr/local/bin/railway-backup-restore-drill

ENV TM_SERVER_PROFILE=cloud-bootstrap \
    TM_SERVER_HOME=/var/lib/tm

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/railway-entrypoint"]
