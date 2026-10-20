# Fuzzing

`fuzz/` holds [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets.
It is a separate package and not a member of the main workspace, so normal
builds and CI do not touch it.

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run open_corrupt
```

`open_corrupt` has two paths. The first feeds arbitrary bytes in as a store
file. The second writes a valid store, flips one byte at an offset the input
chooses, and reopens it. Each path then reads everything it can with `get`,
`contains_key`, `events_since` and `read_record`. The only acceptable outcomes
are `Ok` or `Err`. A panic is a bug.

Last run: 291,000 inputs in two minutes under WSL Ubuntu-24.04 with nightly,
with no failures. That is a short run, not a proof of absence.

Interesting inputs are saved under `fuzz/corpus/` and crashes under
`fuzz/artifacts/`. Both are git-ignored.
