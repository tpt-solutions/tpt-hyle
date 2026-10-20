# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- UniFFI bindings for Kotlin and Swift, built around one object, `SyncStore`.
  It offers `open`, `put`, `get`, `delete`, `export_for`, `import_from` and
  `lamport`.
- The store key is derived from a passphrase. A wrong passphrase is an error,
  and the file is left untouched.
- Writes are signed by the store's node. `add_peer` makes `import_from` refuse
  changes from unlisted nodes or with bad signatures.
- Acknowledgements are signed too, so a relay cannot forge one.

### Known limitations

- The generated Kotlin and Swift sources were checked by reading only. They have
  not been compiled.
- Device packaging (`cargo-ndk` for Android, XCFramework for iOS) is not set up.
