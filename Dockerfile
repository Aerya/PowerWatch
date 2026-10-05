FROM rust:1-bookworm AS builder

WORKDIR /src
COPY . .

RUN cargo build --release -p powerwatch-cli -p powerwatch-tui -p powerwatch-web

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl procps \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/powerwatch /usr/local/bin/powerwatch
COPY --from=builder /src/target/release/powerwatch-tui /usr/local/bin/powerwatch-tui
COPY --from=builder /src/target/release/powerwatch-web /usr/local/bin/powerwatch-web

ENV HOME=/data

EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/powerwatch-web"]
CMD ["--host", "0.0.0.0", "--port", "3000", "--log", "--history-interval", "60", "--nas-mode"]
