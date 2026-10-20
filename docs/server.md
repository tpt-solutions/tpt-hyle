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
3. Get each client's public key. The client runs this on its own store:

       tpt-hyle-cli public-key client.db

   Put the 64 hex digits it prints in the config as `public_key`. The hub
   refuses any change from that client that is not signed by the key.
4. Write a config file (see the example below) and start the server. The store
   passphrase is read from `TPT_HUB_PASSPHRASE`, never from the config:

       TPT_HUB_PASSPHRASE=... tpt-rift-server serve server.json

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
      "public_key": "<64 hex digits from tpt-hyle-cli public-key>",
      "read": ["notes/", "users/"],
      "write": ["notes/", "users/"]
    },
    {
      "sha256": "<hash from new-token>",
      "node_id": "11",
      "public_key": "<64 hex digits from tpt-hyle-cli public-key>",
      "read": ["public/"],
      "write": []
    }
  ]
}
```

- `store` is the hub's store file. The hub also keeps `<store>.acks`, the
  clients' acknowledgements, beside it. Back up both.
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
- **Writes.** A change is accepted only when its node id is the token's
  node id. Changes from other nodes are skipped. A write outside the token's
  write prefixes closes the connection. A change whose signature does not verify
  under the token's `public_key` also closes the connection.
- **Reads.** A client receives only keys matching its read prefixes.
- **Size.** A message larger than `max_message_bytes` is refused.
- **Rate.** Each connection has a token bucket of `messages_per_second`. A
  client that exceeds it is disconnected. Its reconnect logic then backs off.
- **Fan-out.** Each connection polls the hub every `poll_ms` for changes it
  has not sent yet. Clients converge without any extra coordination.
- **Acks.** When a client has caught up, the hub sends it a marker, and the
  client acks it. The hub saves the latest ack per client in `<store>.acks`.
  Once every configured client has acked, the hub refuses replayed changes at
  or below the lowest acked clock. See [ADR 0014](adr/0014-tombstone-collection.md).

## Collecting tombstones

    tpt-rift-server collect server.json

Run this with the server stopped, and with `TPT_HUB_PASSPHRASE` set. It drops
every tombstone that all configured clients have acked, and every value those
tombstones superseded. It rewrites the store file and starts a new generation,
so clients do a full resync on their next connection.

If any configured client has never acked in the current generation, nothing is
collected. The store is still compacted. A client that is gone for good must be
removed from the config first, and its unpushed writes are then lost.

## Demo: two notes clients

`crates/tpt-rift-server/examples/notes.rs` is a small command-line notes app.
It keeps notes in a local store and syncs them with the hub. Use it to check a
deployment end to end. Commands run from the repository root. The examples use
`bash`; on Windows, run them from Git Bash, or use the equivalent PowerShell
syntax.

1. Create a token and config entry for each client. Use two client nodes, 10
   and 11, and the hub on node 1:

       target/debug/tpt-rift-server new-token     # run twice; keep each token
       target/debug/tpt-rift-server hash-token <token>

2. Create each client's store and read its public key. The notes app takes its
   passphrase from `TPT_CLIENT_PASSPHRASE`:

       TPT_CLIENT_PASSPHRASE=alice-pass cargo run -q -p tpt-rift-server --example notes -- \
         alice.db ws://127.0.0.1:8080 10 1 put notes/1 "hello from alice"
       TPT_HYLE_PASSPHRASE=alice-pass target/debug/tpt-hyle-cli public-key alice.db

   Repeat for bob (node 11). Put both public keys and the token hashes in
   `server.json`, with `read` and `write` set to `["notes/"]`. Set `listen` to
   `127.0.0.1:8080`.

3. Start the hub, then sync each client. Each client sends its own token in the
   `TPT_CLIENT_TOKEN` environment variable:

       TPT_HUB_PASSPHRASE=hub-pass target/debug/tpt-rift-server serve server.json &

       TPT_CLIENT_PASSPHRASE=alice-pass TPT_CLIENT_TOKEN=<alice token> \
         cargo run -q -p tpt-rift-server --example notes -- alice.db ws://127.0.0.1:8080 10 1 sync
       TPT_CLIENT_PASSPHRASE=bob-pass TPT_CLIENT_TOKEN=<bob token> \
         cargo run -q -p tpt-rift-server --example notes -- bob.db ws://127.0.0.1:8080 11 1 get notes/1

   Bob reads `hello from alice`. Writes made offline sync the same way:
   `put` saves locally, and `sync` sends the write to the hub and pulls the
   other clients' writes.

`sync` runs several paced rounds. The hub answers on its own poll interval, so
one round can return before the reply arrives. A client whose public key is not
in the config has its writes refused, and the hub logs `event signature does
not verify`. The hub's own log shows a `connection closed` line each time a
client finishes a sync. That is normal.

## Not yet built


- Per-token revocation. Remove the entry from the config and restart.
- Hot reload of the config. A restart is required.
- Compaction on a running hub. Stop the server, run `collect` or the CLI
  `compact`, then start it again.

## Container image

The repository's `Dockerfile` builds a container with the server binary and a
non-root user. The data lives in a volume, and the config is mounted read-only:

    docker build -t tpt-rift-server .
    docker run -d --name tpt-rift -p 127.0.0.1:8080:8080 \
      --env-file hub.env \
      -v tpt-rift-data:/var/lib/tpt-rift \
      -v "$PWD/server.json:/etc/tpt-rift/server.json:ro" \
      tpt-rift-server

`hub.env` holds `TPT_HUB_PASSPHRASE=...`. Keep it out of the image and out of
version control. To collect tombstones, stop the container and run the same
image with `collect`, using the same volume and env file:

    docker run --rm --env-file hub.env \
      -v tpt-rift-data:/var/lib/tpt-rift \
      -v "$PWD/server.json:/etc/tpt-rift/server.json:ro" \
      tpt-rift-server collect /etc/tpt-rift/server.json

The `store` path in the config must sit under `/var/lib/tpt-rift`, which is the
volume.

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
