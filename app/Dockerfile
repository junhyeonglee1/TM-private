FROM rust:1.97-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src-tauri ./src-tauri
RUN cargo build --locked --release -p tm-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates gosu \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home tm

COPY --from=builder /build/target/release/tm-server /usr/local/bin/tm-server
COPY scripts/railway-entrypoint.sh /usr/local/bin/railway-entrypoint
RUN chmod 0755 /usr/local/bin/tm-server /usr/local/bin/railway-entrypoint

ENV TM_SERVER_PROFILE=cloud-bootstrap \
    TM_SERVER_HOME=/var/lib/tm

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/railway-entrypoint"]
