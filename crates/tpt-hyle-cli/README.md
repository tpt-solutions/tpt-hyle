# tpt-hyle-cli

Command-line tool for tpt-hyle stores: inspect, dump, verify, print the public key, and compact.

    TPT_HYLE_PASSPHRASE=... tpt-hyle-cli inspect store.db
    TPT_HYLE_PASSPHRASE=... tpt-hyle-cli dump-sync-log store.db [--json]
    TPT_HYLE_PASSPHRASE=... tpt-hyle-cli verify store.db
    TPT_HYLE_PASSPHRASE=... tpt-hyle-cli public-key store.db
    TPT_HYLE_PASSPHRASE=... tpt-hyle-cli compact store.db

- `inspect` shows the node id, clock, generation, write and tombstone counts,
  live keys, file size, and the public key.
- `dump-sync-log` prints every write in order. Values print as lossy UTF-8.
- `verify` reads and authenticates every frame.
- `public-key` prints the Ed25519 public key. Put it in the server config.
- `compact` drops superseded values and starts a new generation. Peers resync
  from the start afterwards.

The passphrase comes from `TPT_HYLE_PASSPHRASE`, so it stays out of shell history.
Opening a store runs recovery, which removes an uncommitted tail. Run these on
a copy if the file must stay byte-identical.

Part of [tpt-hyle](../../README.md). Status: pre-alpha. Work is tracked in [TODO.md](../../TODO.md) and the design is in [spec.txt](../../spec.txt) (RFC 003).

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
