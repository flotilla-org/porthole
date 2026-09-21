# Windows foreground activation after intervening input

The Windows adapter now retries a denied foreground request once using an empty
mouse input event followed by `SetForegroundWindow`. The event sets no movement,
button, wheel or keyboard flags. This follows the first activation step in
Microsoft PowerToys' `ManagedCommon.WindowHelpers`.

The original failure reproduced on Beaufort with a usable, unlocked desktop:
the same Porthole daemon could alternate between editors while idle but lost
activation eligibility after another process supplied input. A CLI-side
`AllowSetForegroundWindow` call alone failed when the caller was also ineligible.
This is an activation-eligibility problem, not a persistent permission checkbox.

## Native evidence

The probe launches two real test-owned Win32 editors through Porthole. Before
each switch, a separate process sends F24 to the other editor. The fixture has
no F24 action. This gives a repeatable intervening-input condition without
typing into unrelated applications. It is simulated input, not a physical-user
or remote-session test.

The probe verifies the actual foreground HWND, sends distinct text through
Porthole, and reads both native edit controls back with bounded `WM_GETTEXT`.
It waits for input delivery before comparing text. It also checks that activation
does not change cursor position or modifier/mouse-button state.

| Condition | Result |
| --- | --- |
| Original adapter, idle desktop | 6/6 switches and text checks passed |
| Original adapter, intervening input | 0/6 switches; both editors remained unchanged |
| Original adapter, separate input process, caller handoff only | 0/6 switches; handoff rejected |
| Empty-input-assisted caller handoff | 6/6 switches and text checks passed |
| Corrected adapter, separate input process, no caller handoff | 6/6 passed |
| Packaged isolated regression, corrected adapter | 20/20 passed; cursor and modifier/button state unchanged |

The [20-step result](evidence/beaufort-foreground-regression.json) records the
last run. Original probe outputs and build logs remain under
`C:\dev\windows-parity-plan\evidence\beaufort-agent-20260921` and
`C:\dev\windows-parity-plan\evidence\beaufort-foreground-regression`.

The isolated server uses its own named pipe and in-memory policy store. Temporary
identities are revoked and the two editors are closed. The wrapper stops only
the test server it started. The live vessel's Porthole PID 10008, Cleat PID 11612,
Codex PID 12296 and agent entry PID 12884 retained their original start times.
The pre-existing RAD Cleat PID 13804 was also untouched.

## Implementation limits

The adapter first tries ordinary activation. If that is denied, it submits one
empty mouse event and retries once. The existing 500 ms foreground verification,
surface identity validation, input serialization, and foreground check before
each text/key batch remain in place. If input submission or activation fails,
the operation fails clearly before sending target text.

This uses `SendInput` under its normal integrity restrictions. It does not change
foreground timeout settings, elevate the daemon, attach application input queues,
send an Alt key, or move/click the mouse. The empty-event behavior is established
by the referenced implementation and native tests; it is not a Microsoft API
guarantee of unconditional activation on every Windows configuration.

Still unverified: physical intervening input, minimized/maximized applications,
higher-integrity targets, active menus, lock/RDP recovery and SSH reattachment.
The original running vessel still uses the prior daemon binary; this change was
validated on the isolated server, not deployed by restarting that agent's daemon.

## Reproduce

From a usable Windows GUI session, build both native test executables:

```powershell
cargo build -p portholed --example windows_foreground_server --locked
cargo build -p porthole-adapter-windows --example desktop_fixture --locked
powershell -NoProfile -File scripts\windows-vessel\test-foreground.ps1 `
  -ServerExecutable target\debug\examples\windows_foreground_server.exe `
  -FixtureExecutable target\debug\examples\desktop_fixture.exe `
  -EvidenceDirectory C:\dev\foreground-evidence-new `
  -Iterations 20
```

Use a fresh evidence directory. The test exercises the actual native adapter and
HTTP/named-pipe routes. It returns nonzero for wrong foreground, wrong/missing
text, changed input state, or a failed cleanup. No human click is needed.

All four repository gates passed: workspace build, workspace tests, strict
all-target Clippy, and pinned-nightly formatting. PowerShell parser validation
and the native regression also passed. The native probe is separate from headless
CI because it intentionally changes foreground windows on a real desktop.

## Reference implementations

- [PowerToys WindowHelpers.cs at 1716aa14](https://github.com/microsoft/PowerToys/blob/1716aa14ec3b873de97a5ba0b318988e38b8b9a1/src/common/ManagedCommon/WindowHelpers.cs)
  submits an empty mouse event before activation, then has a separate input-queue
  attachment fallback. Only the empty-event pattern is used here.
- [xa11y uia.rs at 7949afc2](https://github.com/xa11y/xa11y/blob/7949afc2cfd6be9c585134d028d71b408fc6caa3/xa11y-windows/src/uia.rs)
  uses `SetForegroundWindow` and reports foreground-lock denial. Its semantic
  accessibility activation does not solve this background-input case.
- [RPA Framework's Windows foreground keyword](https://github.com/robocorp/rpaframework/blob/master/packages/windows/src/RPA/Windows/keywords/window.py)
  calls the available focus and activation methods, then moves the cursor to the
  window's center. Examined on 2026-09-21; not adopted here.
- Microsoft documents the eligibility and expiry rules for
  [AllowSetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow)
  and the integrity restrictions on
  [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput).
