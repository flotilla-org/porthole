# Windows Porthole helper source

This is the first source-tree slice of the Windows console-handoff helper. The
WinUI 3 application stays unelevated. Its single operator action starts a fixed
Rust worker through UAC, authenticates both ends of a bounded named-pipe
exchange, grants foreground access to the retained Porthole daemon, and asks the
worker to transfer the current RDP session to the console. The worker accepts no
caller-selected session or command. A successful transfer leaves the user signed
in and the local desktop unlocked.

The installed prototype at commit `b75c8f1` passed approved and declined UAC.
This source-tree build also passed a native handoff from
`%ProgramFiles%\PortholeHelper`: while RDP was disconnected, Porthole focused,
typed into and captured a test-owned editor. The original workload persisted
after reconnect. See `docs/evidence/windows-helper-20260922-attempt2`.

The helper starts in the notification area. A left click opens a compact WinUI
flyout beside the icon with handoff status and the disconnect action. Escape or
clicking elsewhere dismisses it while the helper stays running. Right click
shows native Show and Quit commands. The handoff keeps the flyout visible while
it needs the operator's attention. A native tray click confirmed that the flyout
opens in front of the caller above the icon, exposes the handoff action, and
dismisses on Escape or focus loss; see
`docs/evidence/windows-helper-flyout-20260923.json`. The installed build passed
the same check and its right-click Quit ended only the helper while Porthole,
Cleat and the agent stayed running; see the adjacent installed and quit evidence
files. This follows Microsoft's [notification-area guidance](https://learn.microsoft.com/en-us/windows/win32/uxguide/winenv-notification):
left click opens a lightweight flyout, right click opens the context menu, and
the flyout closes when focus moves elsewhere. Like the macOS menu bar helper,
it should eventually present the same host concepts as permissions
and agent request review are added; shell and OS operations remain native to
each platform. On Beaufort, the installed merged build also passed focused
keyboard access: Enter on the focused notification-area icon opened the flyout,
its handoff action was keyboard focusable, and Escape dismissed it. Restarting
the Explorer process that owned the taskbar restored the icon without restarting
the helper, Porthole, Cleat, or the agent; the restored icon opened the original
helper. See `docs/evidence/windows-helper-shell-20260923.json` and the
repeatable `scripts/windows-helper/check-tray-shell.ps1`. This is focused
automation acceptance, not a Narrator or full keyboard-navigation audit.

The production release still needs a signed installer and publisher identity,
startup integration, full accessibility acceptance, and adversarial IPC review.
The new reconciliation path still needs an actual uncertain post-commit handoff
test on an operator-coordinated host. Do not distribute
this unsigned development build as a finished helper.

## Development build

Run `./build.ps1` from this directory on Windows with .NET SDK 10 and the pinned
Rust toolchain. It restores locked dependencies, runs the channel identity
check, publishes the self-contained WinUI app, builds the Rust worker, and
records a SHA-256 build inventory. `./check-launch.ps1` checks that the build
starts unelevated with its flyout hidden and closes cleanly. `ChannelChecks --check-desktop`
additionally checks discovery of a live same-session Porthole daemon and its
authenticated API pipe; it does not invoke UAC or disconnect RDP.

`install-dev.ps1` is a reviewed, elevated development copy into
`%ProgramFiles%\PortholeHelper`. The inventory detects accidental build drift;
it is **not** a signature or trust root. Install and handoff require separate
native acceptance on a logged-in test host. Do not replace the installed
prototype at `%ProgramFiles%\PortholeHelperPackagingSpike` or a live daemon.

The helper records bounded public diagnostics under
`%LOCALAPPDATA%\Porthole\helper`. A committed handoff with an uncertain result
remains disabled across helper restarts. **Inspect previous handoff** checks
that no handoff worker remains and the current Windows session is active;
inspection alone cannot prove the prior outcome or enable another transfer.
After checking the desktop and Porthole, the operator may explicitly acknowledge
the attempt. The helper archives its prior journal before re-enabling the action,
and every new attempt still requires RDP, desktop, daemon and UAC checks. A
test-owned synthetic recovery passed without launching an elevated worker or
changing the live workloads; it also rejected a second helper instance. See
`docs/evidence/windows-helper-recovery-20260923.json` and
the focused IPC review in `docs/2026-09-23-windows-helper-ipc-review.md`.
