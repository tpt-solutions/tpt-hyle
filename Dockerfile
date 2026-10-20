# Container image for tpt-rift-server.
#
# Build:  docker build -t tpt-rift-server .
# Run:    docker run -p 127.0.0.1:8080:8080 \
#           --env-file hub.env \
#           -v tpt-rift-data:/var/lib/tpt-rift \
#           -v "$PWD/server.json:/etc/tpt-rift/server.json:ro" \
#           tpt-rift-server
#
# The config inside the container must listen on 0.0.0.0:8080 so that the
# published port reaches it. Publish on 127.0.0.1 only, and terminate TLS in a
# reverse proxy in front of it. See docs/server.md.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p tpt-rift-server

FROM debian:bookworm-slim
RUN useradd --system --home-dir /var/lib/tpt-rift --shell /usr/sbin/nologin tpt \
    && mkdir -p /var/lib/tpt-rift /etc/tpt-rift \
    && chown tpt:tpt /var/lib/tpt-rift
COPY --from=build /src/target/release/tpt-rift-server /usr/local/bin/tpt-rift-server
USER tpt
WORKDIR /var/lib/tpt-rift
VOLUME ["/var/lib/tpt-rift"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/tpt-rift-server"]
CMD ["serve", "/etc/tpt-rift/server.json"]
