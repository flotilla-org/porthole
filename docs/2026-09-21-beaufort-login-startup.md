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
Authenticated desktop-operation failure/recovery across an ordinary RDP
disconnect remains outstanding. The lock/unlock result below records partial
recovery and an activation failure. The console-handoff
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

### Thirty-cycle console run

A follow-up alternated focus, authenticated text input and capture between two
fresh test editors for 30 cycles. Before handoff, six alternating cycles passed
as a connected baseline. The console run then passed all 30 cycles from
`2026-09-21T19:25:37.0154705Z` through
`2026-09-21T19:26:53.0681600Z`, completing cleanup at
`2026-09-21T19:26:55.8547391Z`.

- Session 1 was `console ... Active` both before and after every cycle.
- Every focus ended with the target editor's PID in the foreground. Subsequent
  cycles switched from the other editor; the first switched from PID 13108.
- Every input matched the target's expected accumulated text. After each cycle,
  both editors were checked to catch input delivered to the wrong window.
- All 30 captures succeeded. Visual inspection of the final two PNGs confirmed
  that one editor contained only odd-numbered steps and the other only even
  steps. Desktop availability was reported as granted throughout the probe.
- Both editors closed and the temporary identity was revoked without cleanup
  errors. The original Porthole, agent wrapper and Cleat processes retained
  their start times after reconnect.

Local evidence is in
`C:\dev\windows-parity-plan\evidence\console-reliability-20260921-202508`,
including `result.json`, `step29.png` and `step30.png`. The reusable local probe
is `C:\dev\windows-parity-plan\debug\console-reliability-proof.ps1`.
This is a successful short sustained run after one console handoff, not 30
independent handoffs or a long-duration unattended soak. It strengthens the
evidence for the manually logged-in, no-viewer workflow without explaining the
first experiment's activation failure. The deferred coverage above is unchanged.

## Lock/unlock: explicit failure, partial recovery

On September 21 the operator locked and unlocked the existing Beaufort session.
The authenticated probe used a fresh test editor and temporary identity.

- Baseline focus, text verification and screenshot passed at
  `2026-09-21T20:03:49.2562132Z`.
- At `2026-09-21T20:04:14.0834358Z`, desktop availability was false. Focus and
  capture both failed with HTTP 403 `system_permission_needed`, reporting that
  the interactive input desktop was unavailable (Windows access denied).
  The probe verified that the editor still contained only `before;`.
- At `2026-09-21T20:04:36.2694774Z`, after unlock and a two-second settling
  delay, desktop availability was true. Foreground activation nevertheless
  failed after the 500 ms poll, so text input was not reached.
- The independent capture succeeded after unlock. Visual inspection confirmed
  a valid editor image containing the unchanged `before;` text. Its SHA-256 is
  `121D32BE1C1A6B17A476481ECA2865C7409F1C8B940EFCB861A148410B9AFD5B`.
- Porthole PID 8620, agent wrapper PID 7320 and Cleat PID 14800 retained their
  recorded start times and Windows session. The test editor closed and its
  identity was revoked without cleanup errors.

The overall probe verdict is **FAIL**, because automatic input recovery did not
pass. Process persistence, explicit locked-desktop errors and capture recovery
did pass. The repeated activation error resembles the first console-handoff
failure, but a common root cause has not been established. Do not mark the
desktop-transition acceptance complete on this evidence.

Local evidence: `C:\dev\windows-parity-plan\evidence\lock-unlock-20260921-210348`
contains `result.json`, `process-baseline.json` and `after-unlock.png`.
The local reproducer is
`C:\dev\windows-parity-plan\debug\lock-desktop-proof.ps1`; it requires an
operator-coordinated lock/unlock and exercises real authenticated routes.

## Activation diagnosis: Search open on an unlocked desktop

The timed lock diagnostic observed the desktop become usable at
`2026-09-21T20:12:19.1779291Z`. Activation failed at 2, 5, 10, 20 and 40 seconds
afterward. SearchHost PID 7768 held foreground before and after every attempt;
all five captures succeeded and the editor stayed unchanged. A longer settling
delay did not resolve this run. Evidence:
`C:\dev\windows-parity-plan\evidence\lock-activation-diagnostic-20260921-210746`.

The existing live-daemon foreground regression passed 12 switches after input
from a separate process, so that input condition alone did not reproduce this
failure. A smaller experiment then reproduced the activation failure without
any lock, unlock or RDP transition:

1. Launch a fresh test editor and verify baseline focus, text and capture.
2. Open Windows Search with Win+S and verify SearchHost owns foreground.
3. Request authenticated focus on the editor: the same 500 ms activation failure
   occurs, while independent capture succeeds and SearchHost retains foreground.
4. Dismiss Search with Escape only after confirming it still owns foreground.
5. Repeat focus, text and capture on the same editor: all succeed immediately.

Three independent runs produced that same failure/recovery sequence. Their
artifacts are `C:\dev\windows-parity-plan\evidence\search-activation-20260921-a`,
`-b` and `-c`; the local reproducer is
`C:\dev\windows-parity-plan\debug\search-activation-proof.ps1`. Each run closed
its test editor and revoked its temporary identity without cleanup errors.

This establishes open Windows Search as a reproducible activation-blocking
condition on Beaufort and accounts for the foreground owner observed after
unlock. It does not establish which internal Windows mechanism Search uses, or
prove that the earlier uninstrumented console-handoff failure had this cause.
[Microsoft's activation contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)
requires no active menus and permits the foreground process to disable external
activation; meeting other eligibility conditions is not an unconditional grant.

No production workaround was added. The diagnostic's Escape is an explicitly
controlled experimental intervention, not a proposed generic fallback that
silently dismisses user UI. The helper design should account for shell UI left
open before handoff and report activation failure separately from desktop
availability. Original lock/unlock acceptance remains partial until the agreed
recovery behavior is implemented or demonstrated through the original path.

## Host tools

The operator authorized standard tool installation. `Python.Python.3.13` was
installed per-user through winget (Python 3.13.15); the interpreter is
`C:\Users\rober\AppData\Local\Programs\Python\Python313\python.exe`.
The PR helper now runs with that interpreter rather than the earlier temporary
skill environment. `Gyan.FFmpeg` was installed earlier for the flicker recording.

See the [startup recipe](../scripts/windows-vessel/README.md#porthole-at-gui-login)
for registration, testing and removal commands. Removing registration does not
terminate running processes; inspect their recorded identities before cleanup.
