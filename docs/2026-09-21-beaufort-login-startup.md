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
logon and cold startup remain outstanding. No logout, lock or disconnect was
performed for the registration validation above.

## RDP continuity, September 21

The operator disconnected RDP and reconnected to the same Beaufort session.
A separate observer sampled `/info`, process identity (PID plus start time),
and Cleat's public session inspection about every two seconds. Across all 58
samples, from `2026-09-21T19:04:52.1469799Z` through
`2026-09-21T19:06:48.8280867Z`:

- Porthole PID **8620** and agent wrapper PID **7320** retained their start times.
- Cleat's session remained `running`, using `ghostty`, with leader PID **15888**.
- `/info` returned HTTP 200 throughout. Its `interactive_desktop.granted` value
  was false for 16 samples, first at `2026-09-21T19:05:24.9939746Z` and last at
  `2026-09-21T19:05:55.6426160Z`. It returned to true at
  `2026-09-21T19:05:57.7039411Z` and stayed true through the final sample.

The identity remained `agent_0f5b0bd2f05940daafe3ac394b718bdc`, with recorded
continuity marker `cf818517-cf41-49a3-9983-46648de71f2f`. The observer was then
stopped after verifying its own PID and start time; the workload was untouched.
Local evidence is in
`C:\dev\windows-parity-plan\evidence\rdp-20260921-1905\samples.jsonl` and
`result.json`.

This proves process continuity and desktop-availability recovery through the
operator's RDP interruption. It does not prove authenticated input/screenshot
failure or recovery: those operations were not sampled across this interruption.
A separate lock/unlock test and authenticated desktop-operation failure/recovery
across an ordinary RDP disconnect remain outstanding. The console-handoff
experiment below addresses operating without an RDP viewer after explicit handoff.

## Console handoff: input and capture without an RDP viewer

The operator clarified that manual login is acceptable; automatic login and
boot-to-desktop provisioning can wait. The immediate requirement is to keep
desktop automation usable without a person at the desk or a connected RDP client.

Two native experiments used a temporary authenticated identity and a fresh
`desktop_fixture` editor. The probe approved only its own fixture operations,
kept the token in memory, verified editor text through the native edit control,
and saved a Porthole screenshot. The operator handed Session 1 to the console
using elevated `tscon 1 /dest:console`. The operator's shell required the
`$env:SystemRoot\Sysnative\tscon.exe` path. The probe waited for Session 1 to
become `console ... Active`, then waited another ten seconds and rechecked it.

The first experiment failed at foreground activation, reporting HTTP 403
`system_permission_needed` after the 500 ms activation poll. It completed at
`2026-09-21T19:18:46.6806691Z`; input and capture were not reached. Evidence:
`C:\dev\windows-parity-plan\evidence\console-handoff-20260921\result.json`.

The second experiment recorded desktop availability before acting and attempted
capture independently of focus. It passed at `2026-09-21T19:21:35Z` while Session
1 was the active console session and no active RDP session was listed:

- `interactive_desktop.granted` remained true at the disconnected probe.
- Authenticated focus and text input succeeded; the editor contained exactly
  `before;without-rdp;`.
- A 900 by 420 PNG captured the expected editor text, confirmed by visual review.
  Its SHA-256 is
  `936B59AECCFDEEB36B2B73189FF6F259ACF11036F91C656C3BA14CEC650240FD`.
- The test editor closed and its temporary identity was revoked without cleanup
  errors. Porthole PID 8620, the original agent wrapper PID 7320, and Cleat PID
  14800 retained their start times after reconnect.

Evidence is in
`C:\dev\windows-parity-plan\evidence\console-handoff-independent-20260921-202109`,
including `result.json` and `without-rdp.png`. This demonstrates authenticated
Porthole input and capture without an RDP viewer on this host. The previous
activation failure remains unexplained: one successful repeat is not evidence
of reliable unattended operation. No production code changed between runs.
This does not validate Jackstay streaming, a headless/virtual display setup,
genuine logon-trigger firing, or cold startup.

## Host tools

The operator authorized standard tool installation. `Python.Python.3.13` was
installed per-user through winget (Python 3.13.15); the interpreter is
`C:\Users\rober\AppData\Local\Programs\Python\Python313\python.exe`.
The PR helper now runs with that interpreter rather than the earlier temporary
skill environment. `Gyan.FFmpeg` was installed earlier for the flicker recording.

See the [startup recipe](../scripts/windows-vessel/README.md#porthole-at-gui-login)
for registration, testing and removal commands. Removing registration does not
terminate running processes; inspect their recorded identities before cleanup.
