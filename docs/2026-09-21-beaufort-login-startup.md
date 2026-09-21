# Beaufort per-user Porthole startup

Implements the login-registration portion of the Windows workflow tracked in
[Porthole #118](https://github.com/flotilla-org/porthole/issues/118).

The task starts only Porthole, using the existing user's interactive token and
limited run level. A hidden PowerShell supervisor stays running while the daemon
lives, making Task Scheduler's `IgnoreNew` policy effective. No execution time
limit or battery/idle requirement is applied. Agent launch remains explicit.
`start.ps1 -UseExistingDaemon` reuses only the requested executable in the same
GUI session; the default launch path continues to reject an existing daemon.

## Native validation, September 21

- Registered twice: the second registration reused the exact task definition.
- A different configuration using the same name was rejected without changing
  the task. Literal state paths containing spaces, brackets, ampersands and
  apostrophes survived the encoded task-action boundary.
- Manually started the task and verified same-session reuse of the existing
  `portholed` process. Repeated start retained the same supervisor and daemon.
- Removed the test registration without ending its observed daemon or agent;
  the test then stopped only its verified supervisor PID.
- A mismatched `-UseExistingDaemon` launch failed before creating run state.
- All four repository gates passed using the isolated `startup-target` build
  directory. PowerShell parsing, existing launch-helper checks and the new
  `test-startup.ps1` native task test passed.

The installed task is `Porthole GUI - S-1-5-21-2172235497-3434105306-647022115-1001`.
Its script checkout is `C:\dev\windows-parity-plan\startup`; its executable is
`C:\dev\windows-parity-plan\agent-launch-target\debug\portholed.exe`. Retain
those paths while the registration exists. The task's public state file is
`C:\Users\rober\AppData\Local\Porthole\startup\startup.json`.

The installed task was manually started and reported ready at
`2026-09-21T18:44:02.1689252Z`, reusing daemon PID **8620**, started
`2026-09-21T17:04:12.6326280Z`, in Windows Session **1**. Its supervisor is PID
**14372**, started `2026-09-21T18:44:01.5412025Z`. The existing coding-agent entry
PID **7320** and unrelated RAD Cleat PID **13804** remained running.

Review follow-up narrowed the session-local mutex to discovery/start/readiness,
so a validation supervisor can observe the existing daemon while the installed
supervisor keeps running. The installed supervisor alone was refreshed to PID
**14652**, started `2026-09-21T18:49:38.6936897Z`, and reported ready at
`2026-09-21T18:49:39.2626810Z`. Porthole PID/start time remained identical. The
native test then passed alongside that live registration. An injected readiness
failure also verified that `startup.json` retains the failure reason and daemon
identity without replacing the process. All four gates passed again.

This is evidence of registration, manual task execution and daemon reuse. It is
not evidence that the logon trigger fired, that a cold task start launched a new
daemon, or that a newly launched agent reused the login-started daemon. Genuine
logon, cold startup, and RDP disconnect/lock acceptance remain outstanding. No
logout, lock or disconnect was performed for this validation.

## Host tools

The operator authorized standard tool installation. `Python.Python.3.13` was
installed per-user through winget (Python 3.13.15); the interpreter is
`C:\Users\rober\AppData\Local\Programs\Python\Python313\python.exe`.
The PR helper now runs with that interpreter rather than the earlier temporary
skill environment. `Gyan.FFmpeg` was installed earlier for the flicker recording.

See the [startup recipe](../scripts/windows-vessel/README.md#porthole-at-gui-login)
for registration, testing and removal commands. Removing registration does not
terminate running processes; inspect their recorded identities before cleanup.
