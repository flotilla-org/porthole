# Windows desktop adapter

`portholed` selects `WindowsAdapter` on Windows. Run it in an existing,
unlocked GUI login on the same desktop as the target app. Session 0 services
and an SSH shell in Session 0 cannot drive Session 1. Windows-native cleat
hosting an agent in Session 1 can do so even when managed remotely over SSH.
The adapter checks both session identity and the current input desktop.

This is the local desktop slice for #117. Automatic GUI-login startup and
porthole launching cleat/agents with remote attachment remain #118.

## Run and validate

Build from a Windows PowerShell with Rust/MSVC installed:

```powershell
cargo build --workspace --locked
cargo build -p porthole-adapter-windows --example desktop_fixture --locked
Get-Process -Id $PID | Select-Object Id, SessionId
Get-Process explorer | Select-Object Id, SessionId
powershell -NoProfile -File scripts\manual-windows-smoke.ps1
```

The smoke starts a temporary hidden daemon in that GUI session and launches
a visible, real Win32 editor through `porthole launch`. The editor has an OS
EDIT control and a normal message loop, with no automation interface. Input
arrives through Windows, and the daemon captures the real rendered window.
The fixture avoids single-instance/broker ambiguity and unsaved-document prompts.

The script refuses to run alongside an existing `portholed`. It uses the normal
per-user named pipe, creates a temporary identity through `porthole agents create`,
demonstrates missing-token and missing-grant failures, then approves only that
identity's pending requests using `porthole agents approve`. The token stays in
the script's process environment and is never printed or saved. Focus/input
use `drive`, screenshot/wait use `observe`, and close uses `manage`. The launch
grant applies separately to `launched_by_agent`.

It saves `target/windows-117-evidence/visible-input.png`, command/result evidence
and daemon logs, closes only its test windows, revokes its identity and stops
its own daemon. Inspect the PNG to confirm both lines of input. A second window
exits via native Alt+F4; a subsequent focus must detect native disappearance.
No existing user windows are closed. The script also checks failed executable
launch, a process exiting without a window, an unknown SurfaceId and an unsupported
pixel wait. Policy data is stored in `%LOCALAPPDATA%\Porthole\agent-policy.sqlite`.

For a temporary daemon without the smoke, from the existing GUI login:

```powershell
$validationDaemon = Start-Process .\target\debug\portholed.exe -WindowStyle Hidden -PassThru
.\target\debug\porthole.exe info
# After your validation and test-window cleanup:
Stop-Process -Id $validationDaemon.Id
```

Provision credentials through the same local-trust operator commands before
protected CLI calls. This retains ADR-0006's development authority boundary;
it does not create a new operator/agent security boundary.

## Supported scope and failure behavior

- Process launch honors executable, arguments, environment and working directory.
  Only a unique visible, unowned top-level window belonging directly to the new
  process is accepted, with strong PID correlation. Multiple windows are ambiguous;
  a process exiting without a window fails correlation; no window by the deadline
  yields `launch_timeout` with its PID. A failed correlation does not kill the
  process or close guessed windows. Brokered, descendant-hosted, single-instance
  and artifact launches are outside this slice.
- Window refs carry an HWND and random property cookie in a daemon-specific
  namespace. Liveness checks require the original PID and cookie. Destruction,
  HWND reuse and refs from a previous daemon cannot retarget a tracked surface.
  Properties are removed on adapter drop; Windows discards properties with their
  destroyed windows. Search/attach uses the same identity mechanism.
- Focus restores minimized windows and requests foreground activation. Windows
  may deny this; the adapter returns `system_permission_needed` and asks for a
  user activation instead of bypassing the OS rule. `SendInput` checks foreground
  identity before each batch and checks insertion counts. Keyboard commands use
  Win32 virtual keys/current keyboard layout; literal text uses Unicode UTF-16.
  Calls are serialized within this adapter. User input or other desktop tools can
  still change focus concurrently, so run acceptance without competing input.
- Screenshot uses native `PrintWindow(PW_RENDERFULLCONTENT)` and a top-down
  32-bit DIB, converts BGRA to RGBA and encodes PNG. It captures the window, not
  an arbitrary screen rectangle. Metadata includes logical bounds and window DPI.
  Minimized windows, failed capture and an entirely unrendered buffer return
  `adapter_unsupported`. Some GPU/protected applications do not support this
  mechanism or may render incomplete content; inspect evidence for those apps.
  No Windows Graphics Capture, DXGI, Jackstay, streaming or recording is added.
- `PrintWindow` is synchronous with no cancellation API. One dedicated worker
  owns all GDI resources until it returns; the request times out after three
  seconds. If the worker remains stuck, further screenshots fail explicitly
  instead of spawning more workers. Other daemon operations remain available.
- Close sends `WM_CLOSE` only to the verified window and checks disappearance.
  If the app leaves an unsaved-document prompt or refuses to close, `close_failed`
  is returned after two seconds. No process termination fallback is used.
- Presence/title waits are implemented; stable/dirty waits are rejected through
  the shared wait preflight as `adapter_unsupported`. Pointer operations,
  placement, display enumeration, attention, UIAutomation content rect and
  continuous capture remain unsupported and are not advertised.

Windows enforces session/desktop and integrity restrictions per operation.
An accessible interactive desktop does not promise that every target accepts
input or capture. The adapter reports the native failure; it does not change
login, integrity, foreground-lock settings or desktop permissions.

Native API references: [SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow),
[SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput),
[PrintWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-printwindow).

## Verification

The Windows CI job builds the workspace, runs core/protocol/transport and Windows
adapter tests, and builds the real editor fixture. Adapter tests create hidden
native windows for identity/liveness checks; they never focus or inject input.
They also cover missing executable errors, shared wait rejection and the shared
key-name vocabulary. Interactive acceptance is manual, not claimed from CI.

See [the gouda evidence report](2026-09-07-windows-117-evidence.md) for tested
revisions, commands, PNG, the original Windows test failures, and their fixes.
The full Windows workspace test suite now passes and runs in CI.
