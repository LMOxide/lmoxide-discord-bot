# syntax=docker/dockerfile:1
#   docker build -t lmoxide-discord-bot .
#   docker build --target dist --output type=local,dest=out .    (only the binary, in out/)
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/lmoxide-discord-bot /lmoxide-discord-bot

FROM scratch AS dist
COPY --from=build /lmoxide-discord-bot /

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10002 --home-dir /data bot \
 && mkdir -p /data && chown bot:bot /data
COPY --from=build /lmoxide-discord-bot /usr/local/bin/
USER bot
WORKDIR /data
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["lmoxide-discord-bot", "--config", "/etc/lmoxide-discord-bot/config.toml"]
