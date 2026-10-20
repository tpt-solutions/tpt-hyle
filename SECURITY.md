# Security Policy

## Supported versions

tpt-hyle is pre-alpha. There are no released versions yet, so only the `main`
branch receives security fixes.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem.

Report it privately with GitHub's private vulnerability reporting: open the
repository's **Security** tab and choose **Report a vulnerability**. We will
acknowledge the report, keep you informed, and credit you in the fix if you
want that.

## What counts as a security issue

The store is meant to handle hostile or damaged disk contents. Treat these as
security bugs:

- Any panic, out-of-bounds access, or unbounded allocation triggered by
  crafted or corrupted segment bytes. The goal is that `open` and `get` return
  an error instead.
- Recovery that silently discards committed data, or accepts a state record
  that does not match its sync event.
- Any sync or transport code that accepts an event without verifying its
  signature when a directory of node keys is in use.

Format version 4 seals every frame with XChaCha20-Poly1305 under a key from
an Argon2id passphrase, and signs every event with the issuing node's Ed25519
key. A reader without the passphrase learns no key or value. A tampered frame
fails authentication, and a changed event fails its signature.

These protections have limits:

- The node signing key is derived from the store key. Anyone who can open a
  store can sign as its node. Signatures prove which store wrote an event, not
  which person did.
- The file reveals its size, the number of frames, and when they were written.
- A client can sign a false acknowledgement about its own state. If it does,
  the hub may collect a tombstone that client has not applied. No other node can
  forge an acknowledgement. See [ADR 0014](docs/adr/0014-tombstone-collection.md).
- A replica without a trusted peer does not check signatures.
- Losing the passphrase loses the data. There is no recovery path.

Do not use tpt-hyle as the only protection for data that must stay secret from
someone who can run code on the machine, and do not treat a signature as proof
of a person's action.

## Scope

The fuzz harness in `fuzz/` and the bit-flip tests in
`crates/tpt-hyle/tests/engine.rs` target the first point above.
