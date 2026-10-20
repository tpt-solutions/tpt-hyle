# tpt-rift-server

Reference sync server for tpt-rift. It is a hub: it stores data, accepts
authenticated WebSocket connections, and passes changes between clients.

- Bearer tokens, stored only as SHA-256 hashes. Each token is bound to one node id.
- Key-prefix read and write permissions per token.
- Message size limit and a per-connection rate limit.

```sh
tpt-rift-server new-token
TPT_HUB_PASSPHRASE=... tpt-rift-server serve server.json
TPT_HUB_PASSPHRASE=... tpt-rift-server collect server.json   # server stopped
```

Operator guide, example config, and the TLS requirement: [docs/server.md](../../docs/server.md).
Design: [ADR 0006](../../docs/adr/0006-server-auth-and-access.md).

**Not built yet:** hot config reload. The container image is written (see the Dockerfile) but not built here.
TLS is expected from a reverse proxy.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
