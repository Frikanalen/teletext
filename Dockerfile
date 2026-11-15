# ttxd, the teletext page server, for Kubernetes. The SDI side is ttx-inject, which
# runs on the host with the DeckLink card and is released as a binary instead.
FROM rust:1.97-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY shim shim
RUN cargo build --release --locked --bin ttxd

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 ttxd \
 && apt-get update && apt-get install -y --no-install-recommends tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/ttxd /usr/local/bin/ttxd
USER 10001
ENV TZ=Europe/Oslo
EXPOSE 8080 4243
ENTRYPOINT ["/usr/local/bin/ttxd"]
CMD ["/etc/ttxd/ttxd.toml"]
