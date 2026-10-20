# LAN discovery

Each node sends a 23-byte announcement over UDP, to a broadcast address or to a
known peer. A listener collects announcements and keeps the latest for each
node id.

| Offset | Size | Field   | Notes                          |
|--------|------|---------|--------------------------------|
| 0      | 4    | magic   | `TPTD`                         |
| 4      | 1    | version | `1`                            |
| 5      | 16   | node id | `u128`, little-endian          |
| 21     | 2    | port    | the TCP port the node serves on, little-endian |

The address of a peer is the source address of its packet, combined with the
announced port. A packet of any other length, magic, or version is ignored.

Default port: `47813`.

## Trust

Announcements are unauthenticated. A host on the LAN can announce a node id
it does not own, or point at a port that is not a sync server. Discovery
finds candidates only. Connecting still requires a bearer token, issued by
the server's configuration, so a false announcement cannot open a session.

## Why UDP broadcast, not mDNS

mDNS would add a second protocol stack for one feature. UDP broadcast on a
fixed port needs a few dozen lines, works on every platform that supports
UDP, and does not need a responder. It does not cross subnets. A later
version can add mDNS if it is needed.
