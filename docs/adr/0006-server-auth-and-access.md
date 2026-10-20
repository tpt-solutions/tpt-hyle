# ADR 0006: Server authentication and access control

Status: accepted

## Context

Phase 3 needs a sync server. The TODO asks which authentication scheme to
use (token, JWT or mTLS), and what access control to apply. Both decisions
shape the protocol and the operator's work, so they are recorded here.

## Decisions

### 1. Bearer tokens, stored as SHA-256 hashes

Each client holds a random 256-bit token, sent as `Authorization: Bearer`
when the WebSocket is opened. The server stores only `SHA-256(token)`. A
leaked config therefore does not leak credentials. Tokens are generated with
`tpt-rift-server new-token`, which prints the token once.

The comparison checks every entry in constant time, so timing reveals
nothing about which entry matched.

Rejected for now:

- **JWT.** It adds signing keys, expiry and a verifier that must be kept
  correct, and nothing here needs claims beyond a fixed node id and prefix
  rules, which the config already holds. Revisit it when clients need
  short-lived credentials issued by a separate service.
- **mTLS.** It is the strongest option for fleets, but it requires a
  certificate authority and per-device enrolment, which is too much before a
  first deployment. It remains a candidate for Phase 4, when peers connect
  directly.

### 2. TLS is terminated by a reverse proxy

The server speaks plain WebSocket. Bearer tokens are only safe over TLS, so
the operator must put a TLS-terminating proxy in front of it whenever the
server is reachable from outside a trusted network. Building TLS into the
server would duplicate what that proxy already does well, and it would
hide the expectation that it is needed. The operator guide states it.

### 3. Each token binds one node id

A token maps to exactly one node id. That is what stops a client from
impersonating another. A change whose node id differs from the token's is
skipped. Such a change is either a forward of another client's write, which
the hub already has, or a spoofed id, which must not be applied. The skip is
silent on purpose: a client that forwards events from a mesh is not in
error.

### 4. Key-prefix read and write lists

A token has a list of key prefixes it may read and a list it may write.

- **Write outside the prefixes.** A change from the client's own node whose
  key is not writable closes the connection. This is a protocol violation,
  not a recoverable condition.
- **Read.** Exports to the client include only readable keys. The cursor
  still advances past filtered changes, because they are never readable by
  that client.

Prefixes are byte-wise `starts_with` checks. An empty prefix matches
everything, so `""` means "all keys". An empty list means "nothing".

## Consequences

- Each client needs its own token and node id. Sharing a token would share a
  node id, and two writers under one node id would break the lamport
  uniqueness that conflict resolution depends on.
- Token rotation means removing the old hash and adding a new entry. There is
  no revocation list, because the config is the list.
- Access rules cover key prefixes only. They do not limit how many
  events a client may send. Each connection is rate-limited by a token bucket, so a flood disconnects the sender rather than starving other clients.
- The hub's own node id must differ from every client's, and the server checks
  that at startup.
