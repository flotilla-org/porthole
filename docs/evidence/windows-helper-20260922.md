# Windows helper source-tree acceptance on Beaufort

The application in `apps/windows/PortholeHelper` was built and installed as an
unsigned development build at `C:\Program Files\PortholeHelper`. Its 521-file
first installation and 600-file tray update both matched their SHA-256 build
inventories. The initial installed helper opened unelevated. The installed
prototype in `C:\Program Files\PortholeHelperPackagingSpike` was not replaced.

The first editor observer stopped before handoff: focus and capture succeeded,
but its expected `rdp;` text was observed as `rd`. This is retained in
`windows-helper-20260922/result.json` and `rdp.png`. It did not transfer the
session. The observer closed its editor and revoked its temporary identity.
The test now waits up to two seconds for the test editor to consume queued
keyboard events; it never retries input whose delivery is uncertain.

The second observer passed its connected baseline, then the operator used the
new installed helper and approved UAC. At 2026-09-22 22:25:52 UTC the helper
reported `handoff_ready`, worker `DONE`, and an active console desktop. The
observer focused the test-owned editor, appended `console;`, read back exactly
`rdp;console;`, and saved a PNG while session 1 was `console ... Active` and no
RDP viewer was connected. It closed the editor and revoked the temporary agent
identity without cleanup errors. Evidence is in
`windows-helper-20260922-attempt2/result.json` and `console.png`.

After reconnect, session 1 returned to active RDP. Porthole PID 8620, the
agent wrapper PID 7320, Cleat PID 14800 and the new helper PID 10936 retained
their process start times. Porthole `/info` returned HTTP 200 and granted
interactive desktop permission. See
`windows-helper-20260922-attempt2/reconnect-continuity.json`.

The later tray update was installed separately. Its launch check confirmed an
unelevated process remains running after the status window closes. Windows UI
Automation found its icon in Explorer's hidden-icons flyout and invoked it at
2026-09-22 22:36:02 UTC. The same helper PID 8760 reopened a window titled
`Porthole helper`, then remained alive after the window closed back to the tray.
See `windows-helper-tray-20260922.json` and
`scripts/windows-helper/check-tray-open.ps1`.

A separate UI Automation run opened the icon's right-click menu and found
**Open Porthole** and **Quit helper**. It invoked Quit at 2026-09-22 22:38:02
UTC: helper PID 15916 exited while Porthole 8620, agent wrapper 7320 and Cleat
14800 retained their original start times. Evidence is
`windows-helper-tray-quit-20260922.json`; the check is repeatable with
`scripts/windows-helper/check-tray-quit.ps1`. The helper was then relaunched
and left hidden in the notification area. Explorer restart and accessibility
behavior remain unverified.
