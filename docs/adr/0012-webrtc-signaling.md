# ADR 0012: WebRTC signaling through the sync server

Status: accepted

## Context

Phase 4 adds WebRTC for direct peer-to-peer sync. Before two peers can open a
data channel, they exchange session descriptions, and that exchange needs a
path both can reach. The TODO left the approach open.

## Decision

Relay the session descriptions through the sync server, over the same
authenticated WebSocket the peers already use for sync.

- A client sends a text frame `{"signal": {"to": "<node>", "kind": "offer" |
  "answer", "sdp": "..."}}`.
- The hub relays it to the named node, if that node is connected, as
  `{"signal": {"from": "<node>", "kind": ..., "sdp": ...}}`.
- The hub writes `from` itself. A client cannot choose it.
- Signals to nodes that are not connected are dropped. The sender is told
  nothing, which avoids giving a free directory of connected nodes.
- After the handshake, deltas travel on the data channel. The hub is not in
  that path.

The transport library takes the session descriptions as strings and does not
send them itself. Signaling is therefore a caller concern, and the hub is only
one option.

## Why the hub

- It already authenticates every client, so signals cannot be injected by an
  unknown party.
- It already knows which node is connected where, so no separate directory is
  needed.
- A second signaling service would add a port, a credential, and a failure
  point, for a one-message-pair handshake.

## Limits

- **No ICE servers.** The data channel uses host candidates only, which covers
  peers on one LAN. Peers behind NAT need STUN, and TURN for symmetric NAT.
  Neither is configured. Adding them is a config change, and the relay already
  carries everything a TURN credential would need.
- **Full gathering, not trickle.** Each side waits for ICE gathering to finish
  before sending its description. That costs a little latency. It keeps the
  signaling to two messages per connection.
- **Data-channel message size.** A delta must fit in one message. The sync
  round size limits this.
- **Hub trust.** A relay can see and alter session descriptions. A malicious
  hub could substitute its own peer. Peers that need protection against that
  must verify the peer's identity out of band, or use a DTLS fingerprint
  check that the hub cannot change. Neither is built.

## Verification

- A test in `tpt-rift-server` runs a real WebRTC handshake, with the offer and
  answer relayed through the hub, and then syncs a delta over the data
  channel.
- Signal stamping, unknown-node drops, and the connection surviving an
  unroutable signal are tested separately.
