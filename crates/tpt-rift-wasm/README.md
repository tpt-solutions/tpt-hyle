# tpt-rift-wasm

WebAssembly bindings for tpt-rift. A `Store` is an in-memory replica. It does
no I/O: JavaScript moves the bytes with `export` and `import`.

```sh
wasm-pack build --target nodejs crates/tpt-rift-wasm --out-dir pkg-node
node crates/tpt-rift-wasm/examples/two_stores.cjs
```

Tests run under Node: `wasm-pack test --node crates/tpt-rift-wasm`.

Not built: IndexedDB persistence (it needs an asynchronous storage path) and a
browser demo page. Design: [ADR 0010](../../docs/adr/0010-wasm-bindings.md).

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
