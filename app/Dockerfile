FROM --platform=linux/amd64 rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS builder

WORKDIR /build
RUN rm -rf /etc/apt/sources.list.d/* \
    && printf '%s\n' \
        "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260731T000000Z/ bookworm main" \
        "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260731T000000Z/ bookworm-updates main" \
        "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/20260731T000000Z/ bookworm-security main" \
        > /etc/apt/sources.list \
    && apt-get update \
    && apt-get upgrade --yes --no-install-recommends \
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
RUN rustup component add --toolchain 1.97.0 rustfmt clippy
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY assets ./assets
COPY docs ./docs
COPY src-tauri ./src-tauri
RUN cargo fmt --all -- --check \
    && cargo test --locked --workspace \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo build --locked --release -p tm-server

FROM --platform=linux/amd64 debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS runtime

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN test -s /etc/ssl/certs/ca-certificates.crt \
    && rm -rf /etc/apt/sources.list.d/* \
    && printf '%s\n' \
        "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260731T000000Z/ bookworm main" \
        "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260731T000000Z/ bookworm-updates main" \
        "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/20260731T000000Z/ bookworm-security main" \
        > /etc/apt/sources.list \
    && apt-get update \
    && apt-get upgrade --yes --no-install-recommends \
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
