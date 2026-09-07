# Comte macOS desktop workflow: #115

The target-side launch and client attachment path ran on Comte from kiwi on
2026-09-07. The runnable recipe is in
[scripts/macos-workflow](../scripts/macos-workflow/README.md).

## Environment and startup

Comte was already logged into Robert's GUI session, UID 501, on macOS 26.5.1
(build 25F80). Its home directory is on an external volume:
`/Volumes/MiniHomeX/Users/robert`. Kiwi initiated the SSH connections.

The installed Porthole bundle uses the code from `6f7e5a9`, merged unchanged in
`ff3e36c` by PR #120. It is signed with
`Apple Development: Robert Wittams (DYYMCPD885)`. The bundle identifier changed
from the old installation's `org.flotilla.porthole.dev` to
`work.flotilla.porthole.dev`; Robert granted Accessibility and Screen Recording to
the new identity. Both were confirmed after restarting the daemon. Switching to
the Change Direction certificate is deferred.

The old `/Library/LaunchAgents/org.flotilla.porthole.plist` was disabled for this
user and booted out. Its file was retained. The old app was backed up in
`~/dev/porthole-workflow-115-staging/Porthole-before-115.app`. Opening the new
installed helper registered `work.flotilla.porthole.daemon` through SMAppService.
`launchctl print gui/501/work.flotilla.porthole.daemon` confirmed ServiceManagement
ownership and the new parent bundle identifier. The daemon retains its own job
and bundled native-attach MachService. No new desktop login was created.

Cleat was staged with its matching `libghostty-vt.dylib` from kiwi's fleet
release, source revision `dbeeca61148acf4dc13bf727c8e6ed54aa0aadbb`. Its executable
SHA-256 is `cd5b2211d942d8dbb64f9170c2bf5d62ee4f79f139858d6522bc5db9519fec5a`.
Ghostty was 1.3.1. Robert repaired and logged into Codex 0.153.4 before the run.

## Demonstrated drift and correction

The first recipe placed cleat's runtime below the evidence directory in Comte's
long home path and used a long daemon name. Cleat rejected its Unix socket path;
Ghostty displayed an error dialog and Porthole could not correlate a visible
terminal. Darwin's `sockaddr_un.sun_path` contains 104 bytes, so this directory
layout cannot carry arbitrarily long socket names.

The recipe now creates a private short `/tmp/p115-*` runtime and uses `workflow`
as the isolated daemon name. Both GUI startup and SSH attachment set the same
`CLEAT_RUNTIME_DIR`. Retrying that launch succeeded. The first run's identity was
revoked and its verified Ghostty PID stopped. No unrelated Ghostty instance was
closed. Recordings are copied into the persistent evidence directory at cleanup.

## Launch and attachment evidence

The successful evidence directory is
`~/dev/porthole-workflow-115-run2` on Comte. The sequence was:

1. The operator created the test identity and approved its launch request before
   starting the agent. A private file supplied the token to the agent environment.
2. Porthole launched a fresh Ghostty instance, PID 1000. Its shell started a native
   cleat daemon, PID 1004, and Codex with session leader PID 1005. The native Codex
   child was PID 1016. The terminal launch reported strong `pid_tree` correlation
   with `surface_was_preexisting: false`.
3. A separate kiwi terminal ran `ssh -tt comte` with target-side
   `cleat --server workflow attach --no-create agent` and the explicit runtime.
   The prompt and task submission traveled through that attachment.
4. The client detached and a new client attached with `--no-create`. Inspection
   records `before-detach.json`, `after-detach.json` and `after-reattach.json` all
   show the same running session leader, PID 1005. Attachments change from one
   controller to none and back to one controller. The task history remained.
5. The agent launched `WorkflowEditor.app` through the installed Porthole CLI.
   It returned a fresh surface with strong `pid_tree` correlation, PID 1225.
   The fixture's `editor-result.json` confirms the exact text:
   `Porthole on Comte: input from the cleat agent.`

The client sessions on kiwi were recorded through cleat too. The run uses a real
Cocoa window and OS input, rather than an in-memory adapter.

## Authority limit

The token and launch grant were provisioned before agent startup. Input and
screenshot requests for the new editor required separate operator approvals after
its surface ID existed. The agent stopped on each denial; the operator approved
only requests for the test identity through the existing CLI, then the agent
retried. There was no direct policy-store edit or agent-side approval.

This proves the supervised workflow. It does not meet a stricter interpretation
of #115 requiring all permissions for future windows to be granted before the
agent starts. That remaining decision stays explicit; this report does not claim
unattended authority provisioning or close #115 by itself.

## Screenshot, cleanup and checks

The coding agent saved `editor.png` (23,780 bytes), verified its PNG header and
compared the fixture text exactly. The supervisor copied and visually checked the
screenshot too; the expected sentence is visible. `result.md` records the agent's
checks. The editor and agent remained alive until supervisor cleanup.

Ghostty's AX close path returned `close_failed` during cleanup, including a retry
with close confirmation disabled. The final recipe stops its own terminal
wrapper through a run-local marker and verifies `wait --condition gone` instead.
A separate fresh terminal/cleat lifecycle check passed with this final recipe.
The full agent run's verified terminal process was stopped by the supervisor;
its editor closed through Porthole. Test identities were revoked and token files
removed. The target recording was archived under
`recordings/workflow/sessions/agent/session.cast` in the evidence directory.

All four required workspace gates passed on kiwi. A shared Cargo target directory
was used; the existing self-find integration test also needed
`CARGO_BIN_EXE_porthole` pointing to that directory's CLI because its fallback
hardcodes the workspace-local target path. Python syntax and `git diff --check`
also passed. No Rust implementation changed in this slice.

The stale-process close error reported during cleanup is tracked in
[#122](https://github.com/flotilla-org/porthole/issues/122): `AXWindows` failure
was reported as missing permission even though both grants were still valid.
