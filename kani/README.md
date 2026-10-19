# Kani proofs

Kani is a bounded model checker for Rust. The Phase 1 and Phase 4 proof
targets are:

- segment-offset invariants: `payload_offset + record length` stays within
  the State Segment, for any valid sync log;
- recovery never truncates below the header;
- the sync cursor (`events_since`) only ever returns record-aligned offsets.

Harnesses go in `#[cfg(kani)]` modules inside the crate they check, so they
can reach private items. This directory is reserved for shared harness
configuration and notes. It holds no harness code yet.

```sh
cargo install --locked kani-verifier && cargo kani setup
cargo kani -p tpt-hyle
```

Nothing here runs in CI yet.
