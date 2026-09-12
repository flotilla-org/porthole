# Manual macOS desktop agent workflow

Run the operator script on the target Mac, either locally or over SSH. It uses
Porthole's installed daemon in the existing GUI login. Run the attachment client
from a separate terminal on the client Mac. This is a desktop test, not a CI test.

Prerequisites:

- Install a signed Porthole bundle containing the #89 launch fix. Open the helper
  once to register its launchd daemon. Confirm `porthole info` reports both
  Accessibility and Screen Recording granted. Missing grants stop verification;
  use `porthole onboard`, then restart the daemon after granting Accessibility.
- Ghostty, Python 3, Apple command-line tools, a logged-in coding agent, and a
  native cleat build supporting `launch --tag` and `attach --no-create`.
- Keep cleat's accompanying `libghostty-vt.dylib` in its distribution's `lib/`
  directory beside `bin/`; copying the executable alone is insufficient.

Choose a new evidence directory for each run. For example, from the repository
root on the target:

```sh
python3 scripts/macos-workflow/workflow.py start "$HOME/dev/porthole-workflow-run" \
  --cleat /absolute/path/to/bin/cleat \
  --agent-command 'codex --no-alt-screen -s workspace-write -a on-request'
python3 scripts/macos-workflow/prepare-fixture.py "$HOME/dev/porthole-workflow-run" \
  --source crates/porthole-adapter-macos/tests/fixtures/launch_probe.m
cp scripts/macos-workflow/task.txt "$HOME/dev/porthole-workflow-run/task.txt"
```

The first command provisions a temporary identity and launch grant through the
existing local-trust CLI, then launches Ghostty through Porthole. Ghostty starts a
native cleat daemon and the agent. A private `identity.json` supplies the token to
the agent's environment; do not print, copy into a transcript, or commit it.

The script prints the exact target-side attach command. Run it inside
`ssh -tt TARGET 'PRINTED_COMMAND'` from the client. Keep its explicit
`CLEAT_RUNTIME_DIR`, server and `--no-create`: macOS SSH and GUI environments can
have different default paths, and attachment must not create a replacement agent.
A short private `/tmp/p115-*` runtime avoids Darwin's 104-byte Unix socket pathname
field. The evidence directory can remain under a long home path.

Trust only the newly created test directory when the coding agent asks. Ask the
agent to read `task.txt` and perform its bounded task. The task launches the real
Cocoa editor, types a known sentence, records its result and saves a screenshot.
Approve the agent's sandbox request for the installed Porthole CLI as needed.

Input, observation and management of a newly returned surface can require
additional Porthole grants. When the agent reports `agent_permission_needed`, the
operator runs:

```sh
python3 scripts/macos-workflow/workflow.py approve "$HOME/dev/porthole-workflow-run"
```

This approves pending requests only for the run's identity, using the existing
local-trust operator API. Tell the agent to retry the failed operation. This proof
pre-provisions the token and launch permission; it does not demonstrate unattended
permission for future surfaces. Do not edit the policy database or make the agent
approve its own requests.

For the detach/reattach proof, use the printed runtime and server with cleat:

1. Save `cleat … inspect --json agent` as `before-detach.json` on the target.
2. Run `cleat … detach agent` from another connection. The attached SSH client
   should exit. Save inspection as `after-detach.json`.
3. Attach again from a new client terminal with `attach --no-create agent` and
   save inspection as `after-reattach.json`.
4. Verify the same running `process.leader_pid` in all three records, no
   attachments in the detached record, and a controller after reattachment.
   Confirm the existing task history remains visible.

Inspect `editor.png` and compare `editor-result.json` with the task's expected
text. Preserve daemon registration, OS/build details, Porthole and cleat revisions,
launch responses and the three attachment records in the evidence directory.

The coding agent writes `state.json`, and cleanup trusts its surface list; the
script does not independently verify ownership. Before cleanup, the operator must
compare that list with this run's saved terminal launch response and
`editor-launch.json`, confirming that no unrelated surface IDs were added. This is
a supervised local-trust recipe, not an authorization boundary. Then run:

```sh
python3 scripts/macos-workflow/workflow.py cleanup "$HOME/dev/porthole-workflow-run"
```

Cleanup stops the test cleat session, signals its terminal wrapper to finish,
verifies that terminal is gone, closes the editor surfaces, and archives its
recording into the evidence directory, revokes the identity and removes the token
file. The empty cleat daemon can exit after its normal linger interval. If setup
fails before a surface ID is returned, inspect the error and clean up the verified
run-owned process and identity before retrying. Do not close other terminal
instances. A missing OS grant stops cleanup for user action too.
