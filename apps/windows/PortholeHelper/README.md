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

The helper has a notification-area icon with Open and Quit actions. Closing its
WinUI status window hides it while the helper stays running. Like the macOS menu
bar helper, it should eventually present the same host concepts as permissions
and agent request review are added; shell and OS operations remain native to
each platform. Tray accessibility and Explorer restart behavior still need
native acceptance.

The production release still needs a signed installer and publisher identity,
startup integration, accessibility acceptance, adversarial IPC review, and
an explicit way to reconcile an unknown result after commit. Do not distribute
this unsigned development build as a finished helper.

## Development build

Run `./build.ps1` from this directory on Windows with .NET SDK 10 and the pinned
Rust toolchain. It restores locked dependencies, runs the channel identity
check, publishes the self-contained WinUI app, builds the Rust worker, and
records a SHA-256 build inventory. `./check-launch.ps1` checks that the build
opens an unelevated window and closes cleanly. `ChannelChecks --check-desktop`
additionally checks discovery of a live same-session Porthole daemon and its
authenticated API pipe; it does not invoke UAC or disconnect RDP.

`install-dev.ps1` is a reviewed, elevated development copy into
`%ProgramFiles%\PortholeHelper`. The inventory detects accidental build drift;
it is **not** a signature or trust root. Install and handoff require separate
native acceptance on a logged-in test host. Do not replace the installed
prototype at `%ProgramFiles%\PortholeHelperPackagingSpike` or a live daemon.

The helper records bounded public diagnostics under
`%LOCALAPPDATA%\Porthole\helper`. A committed handoff with an uncertain result
remains disabled across helper restarts until an operator inspects the session.
