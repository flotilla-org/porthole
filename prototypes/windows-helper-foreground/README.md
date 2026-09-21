# Throwaway Windows helper foreground prototype

Question: can a helper close its own menu, own foreground, explicitly grant
Porthole foreground eligibility, and immediately request verified input/capture?

This is a native Windows Forms experiment for Beaufort, not an installed helper
or production recovery mechanism. It uses the existing local daemon and the
test editor built under `C:\dev\windows-parity-plan\agent-launch-target`.
Tokens remain in memory; output contains only public metadata and test images.

From the existing GUI session, with a fresh evidence directory:

```powershell
powershell.exe -NoProfile -STA -File prototypes\windows-helper-foreground\prototype.ps1 -EvidenceDirectory C:\dev\helper-foreground-new
```

Use a normal interactive launch. Do not start it with `-WindowStyle Hidden`:
that hid the first helper window during the initial experiment. The operator
reported seeing only the editor; the same helper window was explicitly shown,
then the operator used its real menu. No input was simulated for the menu click.

Choose **Open helper menu**, then **Test foreground handoff**. The menu closes;
a UI timer allows menu dismissal to finish. The helper activates its own form,
checks that it owns foreground, validates the daemon PID/start time, and calls
`AllowSetForegroundWindow` for that daemon only. It then calls the authenticated
Porthole focus/text/screenshot routes and verifies native editor content.
Closing the helper without choosing the menu item cancels the experiment.

## Observed result

The run completed at `2026-09-21T20:26:57.2762875Z` with `PASS`:

- Helper owned foreground and the foreground grant returned success.
- Porthole focused the test editor and it contained `before;helper-grant;`.
- Capture succeeded; the PNG was visually checked against that text.
- The helper/editor closed and the temporary identity was revoked without errors.
- Existing Porthole PID 8620 retained its recorded start time.

The public result is in `evidence/helper-grant.json`. Original PNGs and logs are
under `C:\dev\windows-parity-plan\evidence\helper-grant-20260921-212454`.
Screenshot SHA-256:
`4040AFB1C20CE35BD52B3CD8819090CF83A086AA31DE805206CC8F23FA7C89BD`.

This is one connected-session end-to-end success. It does not prove that the
grant was necessary, survives subsequent user input/UAC, or solves Search-held
foreground after unlock. No RDP disconnect, elevation, policy change, or
unrelated-UI dismissal occurred in that first run. The follow-up below adds
elevation and console transfer. No production workaround is ratified.

## UAC and console handoff follow-up

Add `-ConsoleHandoff` to the command above. The menu action becomes
**Disconnect RDP - keep automation running**. Approving UAC starts a temporary
elevated worker that validates its own nonzero session and the helper's
PID/start time. It opens a unique session-local event and waits at most 30 seconds.
After the worker is armed, the helper activates its own window, verifies foreground,
grants the unchanged Porthole daemon eligibility, and signals that event. The
worker runs `tscon` for its own session with `/dest:console` and exits. The helper
waits for console-active state, waits ten seconds, then tests input and capture.

The operator ran this sequence on September 21. It passed:

- Worker armed after UAC at `2026-09-21T20:41:44Z`.
- Foreground grant succeeded at `2026-09-21T20:41:45.1760448Z`.
- Worker began handoff at `2026-09-21T20:41:45.1817132Z`; tscon exited 0.
- At `2026-09-21T20:41:55.9031122Z`, Session 1 was `console ... Active`.
  Authenticated focus, exact text verification and capture succeeded.
- The PNG visibly contains `before;console-grant;`. SHA-256:
  `6F0E9D62B1A601C06A4286CA6639703F7AAE65C7D314635D951980ABE499E418`.
- The helper/editor closed, the temporary identity was revoked without cleanup
  errors, and the elevated worker exited. Original Porthole PID 8620, agent
  wrapper PID 7320 and Cleat PID 14800 retained their start times afterward.

Public evidence: `evidence/helper-console.json` and
`evidence/helper-console-worker.json`. Original artifacts remain under
`C:\dev\windows-parity-plan\evidence\helper-console-20260921-214129`.
All four repository gates passed after the prototype update.

This demonstrates one complete menu/UAC/grant/console-transfer sequence; it does
not prove the grant is necessary or guarantee repeated-handoff reliability.
Cancellation, negative-path cleanup and concurrent input still need acceptance.
This script prototype uses user-writable source and evidence paths; it is not
the trust boundary for an installed privileged helper. Production packaging,
caller authentication and tightly bounded authority remain design work. No
persistent elevated service, automatic login, or Search-dismissal fallback was
installed. The helper decision remains open pending the operator's design choice.

[Windows API contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow):
the grantor must already be eligible, and later user input can revoke eligibility.
Related decision: [Windows helper console handoff](https://github.com/flotilla-org/porthole/issues/173).
