# Windows helper handoff boundary: focused review

This review covers the first Windows helper's fixed console-handoff worker and
the new uncertain-outcome recovery path. It is a focused source and native-test
review, **not** the production adversarial sign-off in [#178](https://github.com/flotilla-org/porthole/issues/178).

## Authority and protocol

The unelevated helper starts only the worker at the expected Program Files path
through per-use UAC. The worker independently resolves Program Files from the OS,
checks its own path and elevation, and derives its session from its own process.
Its command line accepts only the helper PID and a rendezvous ID. Those values
are discovery inputs, not authorization.

The worker creates one local named-pipe instance with a logon-SID DACL and
remote clients rejected. Its DACL and the helper's requested access omit
`FILE_CREATE_PIPE_INSTANCE`; Microsoft notes that generic write would otherwise
grant that right. The helper verifies the kernel-reported pipe-server PID is its
retained elevated worker. The worker verifies the kernel-reported client PID is
its retained unelevated helper and rechecks user, logon SID, Windows session,
image path and liveness before commit. The helper requests anonymous SQOS so
the worker cannot impersonate it through this channel. See [named-pipe access
rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)
and [local-only pipes](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes).

The wire operation is four-byte `RDY2` / `CMT2` / result / `ACK1`. No message
contains a command, path or session ID. The worker invokes the fixed transfer
once after its second peer check. Existing tests cover a wrong/replayed commit,
peer revalidation failure and disconnect before commit. This change adds partial
commit and disconnect-after-commit tests: a partial frame never invokes the
operation; a full commit may invoke it once even when the result is lost. The
helper therefore persists `handoff_commit_pending` before sending `CMT2`.

## Uncertain-result recovery

An unconfirmed result remains unresolved across helper restarts. The new
inspection action is read-only: it checks that **no** handoff worker process is
still running and that this helper's Windows session is active through RDP or
at the console. It does not infer that the prior transfer succeeded. A second,
explicit acknowledgement archives the old journal and clears the block; the
normal handoff path still checks the RDP session, desktop, daemon and UAC before
another commit. A session-local mutex prevents two copies of this helper from
running the UI concurrently in one logon. Malformed journals stay blocked until
this same inspection and acknowledgement path completes.

The native test `scripts/windows-helper/check-handoff-recovery.ps1` opened the
flyout with a synthetic unresolved journal, verified a test-owned running
worker-name stand-in blocked acknowledgement, then verified inspection alone
kept handoff disabled after it exited. Explicit acknowledgement archived the
journal and re-enabled the action. A second helper instance exited without
opening another flyout. No elevated worker was launched or transfer
invoked. The script
restored the real journal and installed helper, and the existing Porthole,
agent and Cleat process identities were unchanged; see
`docs/evidence/windows-helper-recovery-20260923.json`.

## Remaining release checks

- The development installer copies unsigned files. Its SHA-256 inventory detects
  drift but cannot establish publisher provenance. A signed, protected install
  and update path must precede production trust in either executable image.
- Exercise the actual pipe with wrong-user, wrong-logon, wrong-session, changed
  image, alternate-administrator UAC, replay, worker death and post-commit
  channel loss. The current policy and in-memory protocol tests do not replace
  those native adversarial cases.
- Inject a stalled **elevated** worker into recovery and verify the same block.
  The native stand-in covers process presence but not elevated worker lifetime.
  Test a real uncertain post-commit handoff on an operator-coordinated host;
  the synthetic UI test does not perform transfer.
- Review the helper's session-local single-instance behavior during upgrade and
  clean install, then repeat native approved/declined handoff acceptance from
  the final signed package.

The journal lives in the operator's user profile and is diagnostic state, not a
privilege boundary. Reconciliation never grants elevation or proves future
desktop activation; every new attempt still requires the normal UAC and peer
checks.
