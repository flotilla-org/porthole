# Beaufort agent launch experiment

For the separate two-window native foreground regression, see
[`2026-09-21-windows-foreground-activation.md`](../../docs/2026-09-21-windows-foreground-activation.md).
`test-foreground.ps1` starts an isolated test server and cleans it up; it does
not replace the running vessel daemon. `foreground-proof.ps1` is its lower-level
probe and also supports caller-handoff comparisons against an existing test run.

Run from the existing Windows GUI login. This packet is still under validation;
it does not install a login task or establish SSH access. Use a dedicated run
directory and workspace. It refuses to replace an existing Porthole daemon.

Prerequisites: built Porthole CLI/daemon and `desktop_fixture` example; functional
Ghostty-enabled Cleat; authenticated Codex CLI. Cleat needs the pending-read EOF
fix described in `docs/2026-09-21-beaufort-agent-evidence.md` for reliable attach.
Fresh-daemon launch also needs the handle-inheritance fix in
[Cleat #220](https://github.com/flotilla-org/cleat/pull/220), otherwise capturing
the launch result can hang before the visible console attaches. See the
[clean-launch evidence](../../docs/2026-09-21-beaufort-clean-launch.md).

```powershell
.\start.ps1 -RunDirectory C:\dev\vessel-run `
  -PortholeBinDirectory C:\dev\porthole\target\debug `
  -CleatExecutable C:\dev\cleat\target\debug\cleat.exe `
  -CodexCommand "$env:APPDATA\npm\codex.cmd" `
  -Workspace C:\dev\vessel-workspace
```

The launcher creates a fresh identity and passes its token only in the process
environment through the named-pipe launch API. Run files contain public identity
and process metadata, not the token. Codex uses the operator's existing login and
full-access execution settings for this authorized acceptance task. Its shell
environment explicitly inherits the token. The run's agent wrapper revokes the
identity when Codex exits normally; forced termination requires explicit operator
revocation with `porthole agents revoke <agent_id>`. Do not treat this prototype
as having crash-safe credential cleanup.
The interactive agent wrapper clears the automation harness's `NO_COLOR` value
before starting Codex. Cleat still supplies the terminal type; no capability is
inferred from the SSH client. An already-running Codex retains its startup
environment, so reattaching alone cannot enable its colors.
Environment narrowing and forced-exit cleanup are tracked in
[issue #159](https://github.com/flotilla-org/porthole/issues/159).

The console-launch grant persists for this identity until revocation. A new run
creates a new identity and does not inherit that grant. Nested child invocations
encode the script call to preserve literal paths across Cleat's `cmd.exe` boundary;
the encoded command contains paths only, never a token. Run
`powershell -NoProfile -File scripts/windows-vessel/test-launch-helpers.ps1` from
the repository root to check path handling and cleanup error preservation without
starting an agent or touching the desktop.

The operator can run `approve-proof.ps1 -RunDirectory ... -Seconds 45` while the
agent runs its assigned proof. It approves only this identity's requests for the
test editor. The agent does not grant itself permissions.

If Windows denies foreground activation, the proof preserves the editor and
writes `agent-desktop-pending.json`. Activate that editor in the GUI, then ask the
same agent to rerun the script. It resumes that surface. Do not interpret the
pending artifact as a passing result. Successful completion writes
`agent-desktop-result.json`, a PNG, and closes the editor.

Repeating `start.ps1` inspects and reuses the existing run; it does not attach a
new local terminal. Run `attach.ps1 -RunDirectory ...` in a console to attach.
An optional `-CleatExecutable` selects a corrected client without restarting the
daemon. Detach with Cleat's Ctrl-] then d sequence.

On failed startup, inspect the public state and any owned processes before
cleanup. The launcher revokes a newly created identity on launch failure, but
retains daemon logs and state. It deliberately does not delete prior runs or
terminate an unrelated session.
