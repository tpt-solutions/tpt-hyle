# Running tpt-rift-server

`tpt-rift-server` is a hub. It stores data, accepts authenticated WebSocket
connections from clients, and passes changes between them. A client writes to
the hub, and the hub forwards the writes to other clients, subject to the
access rules in [ADR 0006](adr/0006-server-auth-and-access.md).

## Setup

1. Choose a node id for the hub, and a distinct node id for each client. Node
   ids are decimal `u128` values. Never reuse one.
2. Create a token for each client:

       tpt-rift-server new-token

   This prints a token and its SHA-256 hash. Give the token to the client and
   keep it out of logs. Put the hash in the config.
3. Write a config file (see the example below) and start the server:

       tpt-rift-server serve server.json

## Example config

```json
{
  "store": "/var/lib/tpt-rift/hub",
  "listen": "127.0.0.1:8080",
  "node_id": "1",
  "max_message_bytes": 1048576,
  "poll_ms": 25,
  "messages_per_second": 200,
  "tokens": [
    {
      "sha256": "<hash from new-token>",
      "node_id": "10",
      "read": ["notes/", "users/"],
      "write": ["notes/", "users/"]
    },
    {
      "sha256": "<hash from new-token>",
      "node_id": "11",
      "read": ["public/"],
      "write": []
    }
  ]
}
```

- `store` is a base path. The hub creates `<store>.sync` and
  `<store>.state.<n>` beside it.
- `listen` should be a loopback address when a reverse proxy sits in front of
  the server.
- Each `read` and `write` entry is a key prefix. An empty list grants nothing.

## TLS: required when exposed

The server speaks plain `ws://`. A bearer token sent over an unencrypted link
can be read by anyone on the path. So when clients connect across a network
you do not fully control:

- Terminate TLS in a reverse proxy in front of the server.
- Have the proxy forward the `Authorization` header. It must not strip it.
- Keep the server bound to loopback, so only the proxy can reach it.

## Behaviour

- **Authentication.** A bad or missing token is refused with `401`, before any
  data flows.
- **Writes.** A change is accepted only when its node id is the token's node
  id. Changes from other nodes are skipped. A write outside the token's write
  prefixes closes the connection.
- **Reads.** A client receives only keys matching its read prefixes.
- **Size.** A message larger than `max_message_bytes` is refused.
- **Rate.** Each connection has a token bucket of `messages_per_second`. A
  client that exceeds it is disconnected. Its reconnect logic then backs off.
- **Fan-out.** Each connection polls the hub every `poll_ms` for changes it
  has not sent yet. Clients converge without any extra coordination.

## Not yet built


- Per-token revocation. Remove the entry from the config and restart.
- Hot reload of the config. A restart is required.
- Compaction on a running hub. Stop the server, compact the store offline with
  the CLI, then start it again.

## Container image

The repository's `Dockerfile` builds a container with the server binary and a
non-root user. The data lives in a volume, and the config is mounted read-only:

    docker build -t tpt-rift-server .
    docker run -d --name tpt-rift -p 127.0.0.1:8080:8080 \
      -v tpt-rift-data:/var/lib/tpt-rift \
      -v "$PWD/server.json:/etc/tpt-rift/server.json:ro" \
      tpt-rift-server

Inside the container, `listen` must be `0.0.0.0:8080`, so that the published
port reaches the process. Publish it on `127.0.0.1` only, and keep the reverse
proxy in front of it.

**Not verified here.** The image was written but not built, because no Docker
daemon was running on the machine where this was done. The first build should
be run by someone with a daemon. The `--locked` build depends on the committed
`Cargo.lock`.

## Signaling for WebRTC

Two authenticated clients can open a direct WebRTC data channel by exchanging
session descriptions through the hub. See
[ADR 0012](adr/0012-webrtc-signaling.md). A client sends a text frame:

    {"signal": {"to": "11", "kind": "offer", "sdp": "..."}}

The hub relays it to node 11, if that node is connected, as:

    {"signal": {"from": "10", "kind": "offer", "sdp": "..."}}

The hub sets `from` itself. Signals to unconnected nodes are dropped silently.
After the handshake, deltas travel on the data channel and do not pass through
the hub. Peers behind NAT need ICE servers, which the server does not configure.
