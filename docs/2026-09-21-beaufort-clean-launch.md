# Beaufort clean agent launch and local continuity

The fresh Portholed Vessel launch now completes through visible terminal attach
and agent-driven native desktop input without a human activation click. This
supersedes the clean-launch gap in the [earlier experiment](2026-09-21-beaufort-agent-evidence.md),
but does not establish SSH, login startup, physical-input interference, or
RDP/lock acceptance.

## Revisions and run

- Porthole: `423cddf2861fd651a4a00c77d642f5c29cd98536` (merged #156, #157, #158).
- Cleat: `148a16f`, containing the pending-read EOF correction from
  [#219](https://github.com/flotilla-org/cleat/pull/219) and the daemon handle fix
  in [#220](https://github.com/flotilla-org/cleat/pull/220).
- Run: `C:\dev\windows-parity-plan\vessel-clean-daemon`.
- Session: `beaufort-clean-daemon/coding-agent`, Windows GUI Session 1.
- Agent: `agent_580b19e4bc2f4158a1c1ed567ee718f0`.
- Agent wrapper PID 3656; Codex PID 5476; Cleat daemon PID 1816; Porthole PID 6928.
- Continuity: `7a7d612e-de0b-4e23-b2bf-f65b49b9b4e4`.

The operator authorized restarting test-owned components. Both preceding agents
exited normally and their identities were observed revoked before daemon cleanup.
The unrelated RAD daemon PID 13804 retained its original start time and Session 1.
This was supervised cleanup, not crash-safe lifecycle automation.

## Observed acceptance

Porthole launched a fresh console with strong process-tree correlation. Cleat
saved `cleat-launch.json` and attached the visible console as controller
`beaufort-local`. The agent inherited its token, ran `desktop-proof.ps1`, and
completed launch, focus, text input, PNG capture and close at
`2026-09-21T16:13:24.1986819Z`. The operator approved only this identity's fixture
requests; the agent did not approve itself. No human editor activation was needed.
An earlier attempt in the newly created workspace required accepting Codex's
first-use workspace trust prompt. The final run reused that trusted workspace;
this is not unattended authentication or first-use provisioning evidence.

Both expected lines were present. The PNG hash is
`261F7F4E45FAD52B3E7960B819BDA2D9618EB863A14CB9479773667045F7578A`, identical to the
[existing screenshot artifact](evidence/beaufort-agent-visible-input.png).
The new public result and continuity snapshot are in
[`beaufort-clean-launch.json`](evidence/beaufort-clean-launch.json).

After the proof, `cleat detach coding-agent` removed the local attachment without
ending the session. Repeating `start.ps1` returned `REUSED` with the same agent
PID and identity. A fresh interactive `attach.ps1` connected as controller and
displayed the completed agent conversation. Ctrl-] then d detached successfully
with exit code 0. A new visible console then reattached to that same agent.
The wrapper PID, Codex PID, daemon PIDs and continuity marker remained unchanged.

## Additional launch defect and correction

The first fresh run on merged Porthole completed the desktop proof, but its
visible terminal had no attachment and `cleat-launch.json` never appeared.
A credential-free reproduction captured `cleat launch --json` into a PowerShell
variable while creating a new daemon and short-lived session. Capture hung after
the launcher process exited; stopping that daemon immediately released the JSON.
Launching against an already running daemon returned normally.

Cleat redirected daemon standard streams to null, but Windows process creation
still inherited other inheritable handles, including the capture pipe.
The correction uses `CreateProcessW` with handle inheritance disabled and a
detached console, while retaining environment inheritance. The native API's
[handle-inheritance contract](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
and [Rust process implementation](https://github.com/rust-lang/rust/blob/1.94.0/library/std/src/sys/process/windows.rs)
explain why standard-stream redirection alone did not isolate the daemon.

The new process-level regression failed before the correction and passed after
it, requiring JSON capture to finish while the daemon is still alive. It also
checks runtime paths containing spaces, brackets and Unicode. The final full
vessel run above validates Ghostty, environment/token propagation and attachment
with the corrected binary, beyond that narrow regression.

## Repeatable entry points

From the repository root, with the named prerequisites built:

```powershell
powershell -NoProfile -File scripts/windows-vessel/start.ps1 `
  -RunDirectory C:\dev\windows-parity-plan\vessel-clean-daemon `
  -PortholeBinDirectory C:\dev\windows-parity-plan\agent-launch-target\debug `
  -CleatExecutable C:\dev\windows-parity-plan\cleat-daemon-bin\cleat.exe `
  -CodexCommand C:\Users\rober\AppData\Roaming\npm\codex.cmd `
  -Workspace C:\dev\windows-parity-plan\workspace-clean-423cddf `
  -Server beaufort-clean-daemon -Session coding-agent
powershell -NoProfile -File scripts/windows-vessel/approve-proof.ps1 `
  -RunDirectory C:\dev\windows-parity-plan\vessel-clean-daemon -Seconds 45
powershell -NoProfile -File scripts/windows-vessel/attach.ps1 `
  -RunDirectory C:\dev\windows-parity-plan\vessel-clean-daemon
```

The first command now reuses this live run. A new launch needs a fresh run
directory and deliberate shutdown of the existing owned Porthole daemon.
Do not delete the run files or replace an unrelated daemon to force a restart.

## Validation and remaining work

Porthole's four required gates passed on the merged source. Cleat workspace
tests, explicit Ghostty build/tests, the Rust-only daemon-capture regression,
and pinned-nightly formatting passed. Strict Windows Clippy still fails on
existing warnings in actor/session-runtime, IPC and PTY code. The generic
non-Unix daemon-termination helper is also still a no-op; the new regression
uses explicit cleanup of its private daemon.

Remaining: kiwi SSH attach, interactive-login startup registration, same-agent
RDP disconnect/lock continuity, physical-input and additional foreground edge
cases, and the environment/forced-exit cleanup work in
[#159](https://github.com/flotilla-org/porthole/issues/159). Cleat #219 and #220
must land before this is a recipe using released/mainline dependencies.
