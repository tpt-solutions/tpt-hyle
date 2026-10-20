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
| `tpt-rift` | `wire::proofs::change_decode_never_panics` | No 128-byte input makes the per-change decoder panic |

Last run: all seven verified, under Kani 0.68.0 in WSL Ubuntu-24.04. The six
`tpt-hyle` harnesses cover the format v4 layout.

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

## Network wire decoder: partly proved

`tpt-rift`'s wire decoder takes bytes from peers. The per-change parser,
`decode_change`, is split out of `Delta::decode` so it can be proved on its
own. `change_decode_never_panics` verifies that no 128-byte input makes it
panic, in about 27 s.

Not yet proved:

- **`Delta::decode` as a whole.** Its outer loop over the change count stays
  unbounded for CBMC. Three harnesses over whole deltas (no panic on 48- and
  128-byte inputs, and a canonical round trip) all failed to verify. Capping
  the count did not help.
- **The per-change round trip** (`encode_change` after `decode_change` gives
  the same bytes). Attempted with keys up to two bytes, then one byte with
  empty values. CBMC ran out of memory both times, so the result is undecided.
  Removed rather than left failing.

Next steps: a smaller encoding of the signature comparison, or proving the
round trip on field values rather than bytes.

## Scope

These proofs cover the parsers that take untrusted bytes, and the bounds
checks in fixed-size storage. They do not cover the whole engine. The replay
and recovery logic is tested by property tests, not proved.
