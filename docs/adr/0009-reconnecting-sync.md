# ADR 0009: Reconnecting sync and triggered rounds

Status: accepted

## Context

`run_until` retried a transport that had already closed. A dropped
connection therefore never recovered. The TODO also asks for rounds that
happen when something changes, not only on a timer.

## Decisions

1. **Reconnect with a fresh transport.** `run_reconnecting` takes a connector
   that opens a new transport. It runs at the start and after each failure,
   with backoff between attempts.

2. **Resend the whole log on each new connection.** After a reconnect the
   peer's cursor is reset. Import is idempotent, so resending is safe. A
   connection that died after `send` returned may have dropped the delta, and
   a full resend covers that without acknowledgements. The cost is bandwidth,
   paid once per reconnect.

3. **A trigger for local writes.** `Trigger` wraps a `Notify`. After a local
   write, the caller notifies it, and the loop runs a round at once rather than
   at its next tick. Several notifications before the loop wakes count as one.

4. **Timer as a backstop.** The interval still runs, so changes that were never
   notified are still exchanged.

5. **Store errors stop the loop.** Retrying a poisoned or damaged store cannot
   help, so the error is returned.

## Consequences

- No acknowledgement protocol exists. A sender cannot tell whether a delta
  arrived. Full resends after reconnect bound the damage, but a delta sent on a
  healthy connection and then lost is only repaired by the next resend. A
  periodic full resend is the fallback if that matters.
- Triggers have no effect on a store that is being written by a different
  process. Each store has one writer, as before.
- A reconnect repeats the work of a full sync, which is linear in log length.
  Large logs should use the cursor-based path, and a later version can
  negotiate cursors with the server.
