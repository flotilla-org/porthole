# Beaufort Windows execution plan

Accepted planning scope: [Windows parity: a persistent Portholed Vessel on Beaufort](https://github.com/flotilla-org/porthole/issues/150). Implementation stays in [Start a Windows GUI-session agent and attach from kiwi through cleat](https://github.com/flotilla-org/porthole/issues/118). This document specifies work and acceptance; none of the commands or checks below are reported as completed here.

## Agreed recipe

Porthole starts at GUI login through an interactive per-user Task Scheduler task. The operator explicitly launches a coding agent through Porthole into a separate named Cleat daemon/session. Kiwi initially uses SSH to run the Windows Cleat attachment client against that existing GUI-session daemon. The SSH client does not own or start the agent. Native remote Cleat clients, including the later Wheelhouse path, follow separately.

Each new agent run receives a fresh Porthole identity/token, retained for that run through reconnect and revoked when the run ends. The coding agent's own account authentication is separate. Preserve the same agent/terminal through detach, route loss, RDP disconnect and lock; desktop actions may fail clearly or wait for a bounded interval. Do not silently restart a failed run. Logout/reboot survival is outside this stage.

Keep Beaufort's existing RAD Cleat and unrelated windows untouched. All implementation branches/worktrees are isolated from the clean sibling checkouts. A named Cleat daemon provides separation from RAD; a Porthole runtime directory does not provide a separate Windows pipe or policy database.

## Packet 1: build environment and local desktop baseline

Owner: Porthole Windows workflow. No dependency on Cleat remote-client development, startup registration or Jackstay graphics.

1. Record current revisions, working-tree state, GUI session identity, existing Cleat PID/start time/path, and whether Porthole is already running. Preserve the baseline for comparison.
2. Inspect the Rustup package and install through winget, as agreed. Record the resolved package/toolchain versions. Existing Visual Studio Build Tools and SDK presence must be verified by a real build, not just inventory.

   ```powershell
   winget show --id Rustlang.Rustup --exact --source winget
   winget install --id Rustlang.Rustup --exact --source winget
   ```

3. In a fresh GUI-session environment, or through verified absolute tool paths, verify Rust's MSVC host and install the required formatter. Do not restart RAD to refresh PATH. Do not change an existing global Rust default unnecessarily; record the selected stable compiler and pin that observed version in the resulting recipe.

   ```powershell
   rustc -Vv
   cargo -V
   rustup show
   rustup component add clippy
   rustup toolchain install nightly-2026-03-12 --profile minimal --component rustfmt
   ```

4. Run the exact repository gates from the implementation worktree, retaining exit codes and build logs, then build the native editor fixture:

   ```powershell
   cargo build --workspace --locked
   cargo test --workspace --locked
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo +nightly-2026-03-12 fmt --check
   cargo build -p porthole-adapter-windows --example desktop_fixture --locked
   ```

5. Inspect the smoke before execution. It uses the normal per-user Porthole pipe and policy store and refuses to coexist with an existing daemon. A conflict requires inspecting ownership and arranging the run; never stop an arbitrary daemon. Run from the existing unlocked GUI login with no competing input. Set a unique evidence directory so earlier evidence is not overwritten:

   ```powershell
   powershell -NoProfile -File scripts\manual-windows-smoke.ps1 -EvidenceDir C:\dev\windows-parity-plan\evidence\beaufort-desktop-<run-id>
   ```

6. Visually inspect the actual PNG. Record its hash, sanitized command/results, source/toolchain versions and session identity. Verify temporary identity revocation and owned window/daemon cleanup; compare the RAD process with the baseline. If the smoke fails, diagnose and record the real failure rather than substituting a mock result.

Acceptance: all four gates and fixture build pass; Porthole reports its Windows adapter; the real native editor contains both submitted lines in a Porthole PNG; token/grant denial and dead/missing-surface checks behave as documented; test-owned resources are cleaned up and RAD remains unaffected. Historical gouda results and compile-only checks do not satisfy this evidence.

Deliverable: a repeatable Beaufort setup/build/smoke recipe and sanitized evidence report with the PNG. Keep token values out of logs and reports. This packet creates no persistent startup registration and proves no remote attachment or native Jackstay behavior.

## Packet 2: explicit Porthole-launched agent

Owner: Porthole workflow, with Cleat-owned defects linked when reproduced. Depends on the local desktop baseline.

Prepare an independently built Cleat and verify coding-agent authentication with its normal supported login. Create a fresh Porthole identity and stage/approve only that run's required grants using existing operator commands. Deliver the token through the launch environment, never arguments, task XML or evidence. A grant targeting a surface is provisioned after that surface exists, before the protected operation.

Choose a terminal launch command that the current Windows adapter can correlate to a fresh verified process tree. Windows Terminal's new-window request is a candidate, not proof of process ownership or token inheritance. Test the actual owning process tree and token arrival without printing the token. If no supported terminal satisfies the contract, report the reproducer and resolve the launch gap before installing startup; do not claim success from a window title match.

The agent uses a stable, separately named Cleat daemon/session. Repeating the explicit launch command finds the same live intended session and does not issue a new identity. Report conflicts rather than adopt an unrelated process or invent a replacement. Record daemon/session/agent identities, process start times and a terminal continuity marker.

Acceptance: the real agent is launched by Porthole in the GUI session, is hosted by the separate Cleat, and itself completes local app launch/input/screenshot. The token reaches that agent; repeated launch preserves it and its processes. Explicit end-of-run cleanup revokes only this run's identity and terminates only owned resources. Unexpected termination recovery must identify and revoke orphaned run identities through supported commands; it must never silently substitute another agent.

## Packet 3: kiwi SSH attachment

Owner: Cleat for attachment fixes, Porthole workflow for the recipe and evidence. Depends on the named GUI-session agent.

From kiwi, use an SSH PTY to run the Windows Cleat attachment client with explicit executable, user/runtime, daemon and session selection. Determine quoting and escape behavior from the actual command and shell. Do not assume the SSH process shares the GUI session; it opens the already-running daemon's pipe. Record whether its logon token can actually access that pipe.

First validate failure cases against disposable test-owned state: wrong/missing daemon, missing session, failed inspection/handshake and denied pipe access. Current `--no-create` alone is insufficient evidence of safe behavior. Required result: clear failure, no agent/daemon creation, no replacement, and no session/marker/PID cleanup caused by route failure. Link reproductions and minimal necessary fixes to [Cleat runtime ownership work](https://github.com/flotilla-org/cleat/issues/122). General native remote-client support and a stdio-to-pipe bridge are not prerequisites for this SSH-client route.

Acceptance: interactive input/output and resize work; detach and reattach preserve the same recorded daemon/session/agent identity and continuity marker. Abrupt SSH loss leaves the agent alive. Reconnection neither replays input nor creates a replacement session. Preserve existing controller ownership semantics. Retain exact commands and results, not a fabricated portable command line.

## Packet 4: persistent Porthole startup

Owner: Porthole workflow. Implement after the explicit launch and attachment paths are understood; automatic agent launch is not part of login startup.

Register a stable, owned per-user Task Scheduler logon task using the interactive token, absolute paths and working directory. Explicitly disable the default execution time limit and select duplicate prevention. Review battery/idle conditions and keep the task's supervised action alive so task instance policy has meaning. Inspect any existing registration before update; refuse a name collision with unrelated work.

Acceptance: register twice without duplicating or replacing live owned processes; verify the Porthole process is in the intended GUI session. Agent launch remains explicit. Removing registration removes only the owned task and does not end unrelated Cleat sessions. A genuine logon-trigger check occurs at an operator-coordinated login; manually starting the task is not evidence that the trigger fired. Do not log out an existing session merely to complete this check.

## Packet 5: lifecycle and final workflow evidence

Owner: Porthole workflow with relevant Cleat fixes. Depends on the preceding packets.

RDP disconnect and desktop lock are separate acceptance cases, performed when coordinated with the operator. Inspect effective disconnected-session/logoff policy; report policies that defeat process persistence without silently changing them. Compare the agent PID/start time, daemon identity, session identity and continuity marker before and after each event.

Acceptance: the same agent/terminal survives both events; desktop actions return a clear failure or bounded wait while unavailable; after the GUI is usable again, the agent demonstrates fresh native input and a new Porthole screenshot. Resize, detach/reattach, SSH route loss and unreachable targets retain their packet 3 guarantees. A recreated process with the same name fails the test.

Publish component versions, install/start/auth/connect/cleanup commands, token-free logs, screenshots/hashes, identity comparisons and the full result matrix. Run exact repository gates for implementation changes. Mark unperformed checks as unverified, and do not close the existing implementation issue until its end-to-end acceptance is met.

## Handoff beyond this stage

[Windows Jackstay graphics research](https://github.com/flotilla-org/porthole/blob/6747978f57208689c8ff305052b35f9c7a103a4d/docs/research/windows-parity-graphics.md) owns the detailed future questions. Preserve host authorization/source selection, Jackstay frame leases and resource lifetime, typed endpoint/capability reporting, and capture generations independent of agent lifetime.

A later graphics map chooses source/consumer pairs, WGC versus duplication, D3D handles and synchronization, adapter compatibility, bounded pools, resize/device-loss retirement and cross-host encoded video. Native remote Cleat consumption for Wheelhouse is also later work. Neither forwarding a byte stream nor producing a PNG proves those capabilities.

The plan is complete when these packets and boundaries are agreed; implementation acceptance remains outstanding. A newly observed incompatibility that requires a product/architecture choice returns as a new decision ticket, rather than being hidden inside an acceptance report.
