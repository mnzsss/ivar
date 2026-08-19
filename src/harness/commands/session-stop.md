---
description: Stop the current ivar session and clean up guards.
---

# Session Stop

`/ivar-session-stop` stops the current ivar session and cleans up guards.

## Steps

1. Verify `IVAR_SESSION_ID` is set. If not, error — no active session.
2. Run, **always naming the session**:
   ```bash
   ivar session stop "$IVAR_SESSION_ID"
   ```
   A session id or a unique prefix of one both work.

   > **Never run bare `ivar session stop` to end your own session.** With no
   > argument it stops *every* session in the hall — every discovery session
   > and every feature's sessions, including executor sessions another
   > feature's `tick` is running. It is a hall-wide teardown, not "stop the
   > current one". Reach for it only when tearing the whole hall down
   > deliberately.

   Stopping a session that is already gone is a no-op, not an error.
3. Unset env vars:
   ```bash
   unset IVAR_SESSION_ID IVAR_FEATURE IVAR_SESSION_PATH
   ```
4. Confirm session stopped. Resume operating from the hall root.
