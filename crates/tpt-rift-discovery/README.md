# tpt-rift-discovery

LAN discovery for tpt-rift peers. Each node broadcasts a 23-byte announcement
(node id and serving port) over UDP. A listener collects them.

Announcements are not authenticated, and they do not need to be. They only say
where a candidate peer might be. Connecting still requires the bearer token
from the server configuration. See `docs/discovery.md`.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
