# Bounded Sakura candidate, Linux/amd64. Do not promote until compatibility gates pass.
FROM rust:1.98.1-bookworm@sha256:c49256cbe5ea0188bc658a689500d70c41eb51f009a7a7be209caf60a944f3ec AS build
RUN apt-get update && apt-get install -y --no-install-recommends git cmake clang libssl-dev pkg-config build-essential \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.lock Cargo.toml /app/
COPY src /app/src
COPY tests /app/tests
COPY migrations /app/migrations
RUN cargo test --locked && cargo build --release --locked

FROM debian:bookworm-slim@sha256:f3034a6ec3c1205360777c4aae76234998866ad18806ae62b63a3f84ccad782b AS indexer
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/target/release/indexer /app/indexer
CMD ["/app/indexer"]
