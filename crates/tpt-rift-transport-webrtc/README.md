# tpt-rift-transport-webrtc

WebRTC peer-to-peer `Transport` for `tpt-rift`. One data channel carries one encoded delta per message.

Connecting takes an offer and an answer, which the library does not send. Carry them over any signaling path, such as the sync server's WebSocket or a QR code. Host candidates work on one LAN. STUN and TURN servers (`stun_urls`, `turn`) cover peers behind NAT.

A delta must fit in one data-channel message, so keep the number of events per sync round small.

Part of [tpt-hyle](../../README.md). Status: pre-alpha. Work is tracked in [TODO.md](../../TODO.md) and the design is in [spec.txt](../../spec.txt) (RFC 003).

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
