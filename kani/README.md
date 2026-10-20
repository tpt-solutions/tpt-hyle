# Kani proofs

Kani is a bounded model checker for Rust. It does not run on Windows, so the
proofs are run in a Linux environment, such as WSL.

## Harnesses

Harnesses are `#[cfg(kani)]` modules in the crates they check, so they can
reach private items.

| Crate | Harness | Claim |
|---|---|---|
| `tpt-hyle` | `format::proofs::decode_record_never_panics` | No 56-byte sync record makes decoding panic |
| `tpt-hyle` | `format::proofs::check_header_never_panics` | No 32-byte header makes validation panic |
| `tpt-hyle` | `format::proofs::decoded_data_records_have_valid_checksums` | A record that decodes as data has a matching CRC |
| `tpt-hyle` | `storage::proofs::fixed_read_stays_in_bounds` | A successful read lies inside the written region |
| `tpt-hyle` | `storage::proofs::truncate_never_grows` | Truncation never grows a segment, and a refused one changes nothing |

## Running

On Linux or macOS:

```sh
cargo install --locked kani-verifier
cargo kani setup
cargo kani -p tpt-hyle
```

On Windows, use WSL:

```sh
wsl -d Ubuntu-24.04 -- bash -lc 'cd /mnt/d/<path-to-repo> && cargo kani -p tpt-hyle'
```

Set `CARGO_TARGET_DIR` to a Linux path inside WSL. Otherwise the build
artifacts clash with the Windows build.

## Scope

These proofs cover the parsers that take untrusted bytes, and the bounds
checks in fixed-size storage. They do not cover the whole engine. The replay
and recovery logic is tested by property tests, not proved.
