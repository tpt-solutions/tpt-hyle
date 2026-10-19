# tpt-rift-transport-ws

WebSocket `Transport` for `tpt-rift`, client and server halves.

- `connect(url)`: client, for `ws://` and `wss://` URLs.
- `accept(listener)`: server, one connection per call.
- Each binary message carries one encoded delta. A malformed message closes the link.

**No authentication and no TLS yet.** The auth scheme and access rules are open
decisions (TODO.md, Phase 3). Use it for trusted links and tests only.

Status: transport implemented and tested over loopback TCP. Auth, TLS, and the
reference server are not built.

Part of [tpt-hyle](../../README.md). Work is tracked in [TODO.md](../../TODO.md).

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
