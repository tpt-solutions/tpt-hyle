# ADR 0015: Signed events

Status: accepted

## Context

Until now an event was trusted because of the connection it arrived on. The hub
checked that a client wrote under its own node id, and clients trusted whatever
the hub relayed. That is enough while the hub is the only route. It is not
enough for two cases the design already has:

- **A relay forwards events it did not write.** A client that receives another
  client's event through the hub cannot tell whether the hub altered it.
- **Peers sync directly over WebRTC.** There is no hub to vouch for an event.

Signatures make authorship a property of the event, so any path can carry it.

## Decision

Every data frame carries an Ed25519 signature by its issuing node, and every
wire change carries the same signature. The format is in
[docs/format.md](../format.md#signatures) and [docs/wire.md](../wire.md).

- **Keys are derived, not stored.** A node's signing seed is SHA-256 over a
  fixed label, the store key, and the node id. It is the same on every open, and
  nothing new is stored. Its public key is `Hyle::public_key()`, which
  `tpt-hyle-cli public-key` prints.
- **The message is domain-separated and unambiguous.** It is a fixed label, then
  the node id, lamport, tombstone flag, and length-prefixed key and value.
- **Verification is strict.** `verify_strict` refuses non-canonical signatures
  and weak keys.
- **The hub checks each client change** against the public key in that client's
  token entry. A bad signature closes the connection, as a write outside the
  token's prefixes does.
- **A replica with a directory checks every change** it imports. A change from an
  unlisted node, or with a bad signature, refuses the whole delta. Nothing in it
  is applied. Without a directory, signatures are carried but not checked.
- **Relays keep origin signatures.** Compaction and forwarding copy the stored
  signature. They never re-sign.

## Consequences

- **Format version 4 and wire version 3.** Both are incompatible with earlier
  versions. Old stores must be re-created from their source, since there is no
  migration.
- **Every write now costs a signature, and every received change costs a
  verification.** Ed25519 is cheap but not free. The numbers are in
  `docs/benchmarks.md`.
- **Signing does not stop a passphrase holder.** The node key comes from the
  store key, so anyone who can open a store can sign as its node. Signatures
  prove which store, and so which node, wrote an event. They do not prove which
  person did. That is the limit of this design, and it is stated in the format
  doc.
- **The directory is distributed out of band.** The hub checks clients through
  the config. A client checks other nodes only if the operator gives it a
  directory. The hub does not publish one. A client with no directory trusts the
  hub to relay honestly, as before.
- **The hub does not verify acks.** Signed events do not sign acknowledgements,
  so a lying client can still defeat tombstone collection
  ([ADR 0014](0014-tombstone-collection.md)). Signing acks is a possible next
  step.

## Alternatives rejected

- **Random per-node keys stored in the file.** Simpler to rotate, but the file
  then holds a secret that the passphrase must protect. A derived key needs no
  storage, and its strength is the store key's.
- **Signing the frame, not the event.** A frame signature binds the nonce and
  the file position. Compaction then has to re-sign, which would break origin
  signatures. Signing the event keeps them stable.
- **One signature per batch.** Cheaper, but a batch of events from two nodes
  would need multi-party signing. Not worth it for the first version.
- **A certificate authority.** Keys would be issued and revoked centrally.
  Unnecessary while the operator already writes the config.
