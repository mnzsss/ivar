---
description: Stop the current ivar session and clean up guards.
---

# Session Stop

`/ivar-session-stop` stops the current ivar session and cleans up guards.

## Steps

1. Verify `IVAR_SESSION_ID` is set. If not, error — no active session.
2. Run:
   ```bash
   ivar session stop
   ```
   (Pass a specific session id or prefix when it is not the most recent
   session on the current feature: `ivar session stop <id-prefix>`.)
3. Unset env vars:
   ```bash
   unset IVAR_SESSION_ID IVAR_FEATURE IVAR_SESSION_PATH
   ```
4. Confirm session stopped. Resume operating from the hall root.
