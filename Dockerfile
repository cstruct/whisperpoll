# syntax=docker/dockerfile:1

# Build the WASM client with trunk and the server, in one Rust toolchain.
FROM rust:1.97-bookworm AS build
ARG TRUNK_VERSION=0.21.14
RUN rustup target add wasm32-unknown-unknown \
 && curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-gnu.tar.gz" \
    | tar -xz -C /usr/local/bin trunk
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cd crates/web && trunk build --release && cd ../.. \
 && cargo build --release -p whisperpoll-server \
 && cp target/release/whisperpoll-server /usr/local/bin/

FROM debian:bookworm-slim
COPY --from=build /usr/local/bin/whisperpoll-server /usr/local/bin/
COPY --from=build /src/crates/web/dist /app/dist
ENV HOST=0.0.0.0 PORT=8080 WHISPERPOLL_DIST=/app/dist
EXPOSE 8080
CMD ["whisperpoll-server"]
