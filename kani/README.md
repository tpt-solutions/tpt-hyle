# Kani proofs

Kani is a bounded model checker for Rust. It does not run on Windows, so the
proofs are run in a Linux environment, such as WSL.

## Harnesses

Harnesses are `#[cfg(kani)]` modules in the crates they check, so they can
reach private items.

| Crate | Harness | Claim |
|---|---|---|
| `tpt-hyle` | `format::proofs::header_decode_never_panics` | No 128-byte header makes decoding panic |
| `tpt-hyle` | `format::proofs::frame_header_decode_is_bounded` | An accepted frame header has a length in [16, 2^26] and a known kind, so nothing is read or allocated past those bounds |
| `tpt-hyle` | `format::proofs::data_decode_never_panics` | No data plaintext makes decoding panic, and an accepted one has exact length |
| `tpt-hyle` | `format::proofs::commit_decode_never_panics` | No commit plaintext makes decoding panic |
| `tpt-hyle` | `storage::proofs::fixed_read_stays_in_bounds` | A successful read lies inside the written region |
| `tpt-hyle` | `storage::proofs::truncate_never_grows` | Truncation never grows storage, and a refused one changes nothing |

Last run: all six verified, under Kani 0.68.0 in WSL Ubuntu-24.04, for the
format v4 layout.

## Running

On Linux or macOS:

```sh
cargo install --locked kani-verifier
cargo kani setup
cargo kani -p tpt-hyle
```

On Windows, use WSL:

```sh
wsl -d Ubuntu-24.04 -- bash -lc 'cd /mnt/d/<path-to-repo> && CARGO_TARGET_DIR=$HOME/tpt-kani-target cargo kani -p tpt-hyle'
```

Set `CARGO_TARGET_DIR` to a Linux path inside WSL. Otherwise the build
artifacts clash with the Windows build.

## Scope

These proofs cover the parsers that take untrusted bytes, and the bounds
checks in fixed-size storage. They do not cover the whole engine. The replay
and recovery logic is tested by property tests, not proved.
