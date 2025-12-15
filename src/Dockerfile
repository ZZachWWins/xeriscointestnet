FROM rust:1.80 AS builder

WORKDIR /usr/src/xrs
COPY . .
RUN cargo build --release

FROM ubuntu:22.04
RUN apt-get update && apt-get install -y libssl-dev ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/src/xrs/target/release/xrs /usr/local/bin/xrs
COPY config.toml /etc/xrs/config.toml
EXPOSE 4000 8081
CMD ["xrs", "--config", "/etc/xrs/config.toml"]