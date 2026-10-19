# Fuzzing

`fuzz/` holds [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets.
It is a separate package and not a member of the main workspace, so normal
builds and CI do not touch it.

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run open_corrupt
```

`open_corrupt` feeds arbitrary bytes into a store as both segments, then runs
`open`, `get`, `events_since` and `read_record`. The only acceptable outcomes
are `Ok` or `Err`. A panic is a bug.

Interesting inputs are saved under `fuzz/corpus/` and crashes under
`fuzz/artifacts/`. Both are git-ignored.
