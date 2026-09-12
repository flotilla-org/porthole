# Windows GUI-session workflow

This recipe uses an existing unlocked Windows login. Porthole runs as that
user with limited privilege; it does not create a login or use Session 0.
Use a native cleat build and `codex.cmd`. PowerShell runs with the machine's
existing execution policy; this recipe does not bypass or change it.

Build the workspace first. Register the daemon for the current user's login:

```powershell
.\startup.ps1 Install -DaemonPath C:\path\to\portholed.exe
.\startup.ps1 Start
.\startup.ps1 Status
```

The task uses an interactive user principal and an at-logon trigger. `Start`
requests an immediate run using the same registration. `Remove` stops and
unregisters only the recognized Porthole task; it does not remove cleat sessions
or files. Stop the task before rebuilding its executable.

```powershell
.\startup.ps1 Remove
```

For a new supervised run:

```powershell
.\workflow.ps1 Start -Directory C:\dev\porthole-run `
  -Cli C:\path\to\porthole.exe -Cleat C:\path\to\cleat.exe `
  -AgentCommand C:\Users\USER\AppData\Roaming\npm\codex.cmd
.\workflow.ps1 Status -Directory C:\dev\porthole-run
```

Credentials are written only after creating a directory restricted to the
current user's SID. The existing local-trust operator API creates a run identity
and approves its launch request. `agent.ps1` supplies the token to the agent's
process environment. Never print or commit `identity.json` or an environment
dump. This is supervised same-user development authority, as in the macOS
recipe; future application surfaces still need explicit action grants.

The printed cleat runtime, server, and session identify the intended endpoint.
Use them for remote `attach --no-create`, inspect, and detach. A missing endpoint
must fail; do not launch a replacement daemon from SSH.

```powershell
.\workflow.ps1 Approve -Directory C:\dev\porthole-run
```

Approval is scoped to pending requests for this run's identity. After the
agent has closed its own test applications, inspect the recorded terminal
launch and run:

```powershell
.\workflow.ps1 Cleanup -Directory C:\dev\porthole-run
```

Cleanup ends the owned agent session, signals its terminal wrapper to exit,
waits for that recorded surface to disappear, then revokes the identity and
removes the token file. Recordings remain under the run directory. If launch
failed without a terminal SurfaceId, cleanup refuses to guess ownership;
reconcile its exact process IDs before closing anything.

## Validation boundary

Gouda passed the supervised terminal/cleat/Codex/editor workflow, including
remote detach/reattach with the same agent and visibly verified screenshots.
See [the evidence report](../../docs/2026-09-11-gouda-desktop-workflow.md).
The initial root-PID-only correlation failed for a console whose visible window
belonged to its child process. The Windows adapter now verifies descendant
process identities before accepting that window.

This recipe selects the classic console host with `-ForceNoHandoff`, present in
[Microsoft's console argument parser](https://github.com/microsoft/terminal/blob/main/src/host/ConsoleArguments.cpp).
It does not modify the user's default terminal. General Windows Terminal broker
correlation is not established by this test.

Registration, immediate start, same-registration idempotence, removal, and
reinstallation have been exercised. Actual at-logon triggering still requires
a coordinated sign-out/sign-in. New editor surfaces required supervised
operator grants after launch, as with the macOS recipe. This is not proof of
unattended pre-authorization for every future surface, and #118 remains open.

## Acceptance procedure

Supply the agent with `task.txt` and the absolute path to the built
`desktop_fixture.exe`. Inspect the first PNG for the exact requested text.
Record `cleat inspect --json agent` before detach, after detach, and after a
fresh SSH `attach --no-create`. Require the same leader PID, no controller
while detached, and a controller after reattachment. Record the cleat daemon
PID and Windows SessionId too; terminal visibility alone is not launch-context
proof. Resume the same agent, inspect its second PNG, and require successful
closure of the editor before recipe cleanup.

Record binary hashes, Windows build, startup task principal/trigger, actual
process context, launch responses, script outputs and any failures. Verify
removal/reinstallation of this run's startup registration without changing
other tasks. An actual sign-out/sign-in test requires user coordination; an
immediate `Start` pass does not by itself prove the at-logon trigger fired.
