# tpt-rift-uniffi

UniFFI bindings for tpt-rift. One object, `SyncStore`, is a file-backed replica
with `open`, `put`, `get`, `delete`, `export_for`, `import_from` and `lamport`.
Transport is left to the app, as on the web.

`open(path, node_id, passphrase)` derives the store key from the passphrase. A
wrong passphrase is an error, and the file is left untouched. Losing the
passphrase loses the data.

Every write is signed by the store's node. Trust is opt-in: call
`add_peer(node_id, public_key)` with a peer's `public_key()`. Once a peer is
trusted, `import_from` refuses changes from nodes not listed and changes whose
signature does not verify. Trust is not saved. Add peers again after each
`open`.

Acknowledgements are signed by the store's node too, so a relay cannot forge
one.

## Generated sources

The Kotlin and Swift sources are in `bindings/`. They are generated from the
compiled library:

    cargo build -p tpt-rift-uniffi
    cargo run -p tpt-rift-uniffi --bin uniffi-bindgen -- generate \
      --library target/debug/tpt_rift_uniffi.dll --language kotlin \
      --out-dir crates/tpt-rift-uniffi/bindings/kotlin
    cargo run -p tpt-rift-uniffi --bin uniffi-bindgen -- generate \
      --library target/debug/tpt_rift_uniffi.dll --language swift \
      --out-dir crates/tpt-rift-uniffi/bindings/swift

On macOS and Linux the library is `libtpt_rift_uniffi.dylib` or `.so`.

## Verification status

- The Rust surface is tested: two devices syncing, data surviving a reopen, and
  malformed input returning errors rather than panicking.
- The generated Kotlin and Swift code was produced and its API checked by
  reading it. It was **not compiled**. This machine has no Kotlin or Swift
  compiler, no Xcode, and no Android SDK.
- Building for devices (`cargo-ndk` for Android, an XCFramework for iOS) is
  not set up.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
