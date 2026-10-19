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
- Any future sync or transport code that accepts an event without verifying
  it.

The current format uses CRC-32C. That detects corruption, not tampering. It
does not protect against an attacker who can rewrite the files. Encryption at
rest and signed events are open questions in [TODO.md](TODO.md). Until they
are answered, do not rely on tpt-hyle to keep data confidential or to prove
its origin.

## Scope

The fuzz harness in `fuzz/` and the bit-flip tests in
`crates/tpt-hyle/tests/engine.rs` target the first point above.
