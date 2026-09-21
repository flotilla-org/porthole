# Beaufort desktop prerequisites and GUI startup options

Research date: 2026-09-21. Resolves the investigation in [Establish Beaufort desktop prerequisites and GUI startup options](https://github.com/flotilla-org/porthole/issues/151). This is planning evidence, not a live desktop acceptance result. No builds, installations, startup registration, desktop input, lock/disconnect, credential changes or agent launches were performed.

## Findings

The current shell and existing RAD Cleat are in Beaufort's interactive Session 1. A repeatable Rust build environment is not established in this shell. Porthole has a native desktop adapter, but Beaufort has no newly demonstrated launch/input/screenshot result. A per-user interactive Task Scheduler logon task is a suitable startup candidate; the terminal launch chain and token handoff still need an explicit decision and live proof.

An important documentation correction: the current Windows adapter accepts a unique window from a **verified descendant process tree**, not just the direct child. The Windows guide still describes the older restriction. Brokered/preexisting windows remain unsupported. See [current launch implementation](../../crates/porthole-adapter-windows/src/native.rs), particularly `LaunchTree`, `valid_birth`, `spawn_process` and `launch_process`; compare the [older Windows guide](../windows-desktop.md).

## Read-only Beaufort observations

Source: probes executed in this research session on 2026-09-21. These observations are transient, not promises about future sessions. Reproduce using the commands below; they do not print credentials.

| Item | Observed |
| --- | --- |
| Host / user | `beaufort` / `beaufort\rober` |
| Windows | DisplayVersion `25H2`, build `26200.8653`, read from CurrentVersion registry values |
| Shell | PowerShell PID 13608, Session 1 at the initial probe |
| Existing desktop | Explorer PIDs 4664 and 6468, Session 1 |
| Existing RAD Cleat | PID 13804, Session 1, `C:\dev\rad-test-tools-kJTJhU\cleat.exe`; left untouched |
| Porthole daemon | No `portholed` process observed |
| PATH | Git, GitHub CLI, OpenSSH, Node/npm and Codex resolve; `cargo`, `rustc`, `rustup`, `cl`, `clang`, `link`, `cleat`, `porthole`, `portholed` did not resolve |
| Rust standard user location | No `C:\Users\rober\.cargo\bin` found; this does **not** establish that Rust is absent everywhere |
| Native tools | `vswhere` reports VS Build Tools 2022 `17.14.41`, including `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`; SDK include directory `10.0.26100.0` exists. No compile/link test performed |
| GitHub authentication | Active account `rjwittams`, keyring-backed, HTTPS Git; scope names include `repo`, `read:org`, `workflow`, `gist`; issue reads succeeded. Private Andamento access was separately verified by the coordinating preflight |
| Coding agent | Codex launcher resolves at `C:\Users\rober\AppData\Roaming\npm\codex.ps1`; standard `.codex\auth.json` exists. Contents were not read. Existence is not a successful authenticated agent request |

The original Porthole, Cleat and Jackstay checkouts all had empty `git status --short` output at inspection. Their respective HEADs were `92b00db93020ea4e610d06a61b2bfef3c3e54e7c`, `d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc`, and `a1e1420d137394f494e156aa1f3f34d996753f9c`. This report's isolated Porthole branch started at that same Porthole HEAD. Neither the running RAD binary's revision nor a working Porthole binary was inferred from these checkout revisions.

Representative reproducible probes:

```powershell
hostname
whoami
Get-Process -Id $PID | Select-Object Id,SessionId
Get-Process explorer,cleat,portholed -ErrorAction SilentlyContinue | Select-Object Id,ProcessName,SessionId,Path
Get-Command rustc,cargo,rustup,git,gh,ssh,codex,cl,cleat,porthole,portholed -ErrorAction SilentlyContinue | Select-Object Name,Source
Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' | Select-Object DisplayVersion,CurrentBuild,UBR
& 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe' -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
git -C C:\dev\porthole status --short
git -C C:\dev\porthole rev-parse HEAD
gh api user --jq .login
```

## Desktop and launch contracts

`WindowsAdapter::desktop` checks a nonzero Windows SessionId and compares the thread desktop with the current input desktop. Failure returns `system_permission_needed`; it does not switch desktops. Focus, input, launch and screenshot use this check. Startup merely warns about missing system permissions and continues serving. This supports separating agent lifetime from desktop availability; lock/disconnect persistence has **not** been tested here. [Adapter](../../crates/porthole-adapter-windows/src/native.rs), [daemon startup](../../crates/portholed/src/main.rs).

Launch passes arguments, environment and cwd to a native child process. It retains the root process handle, discovers descendants using creation/exit times and process handles, and accepts exactly one visible unowned top-level window belonging to a verified live process. It rejects ambiguous windows, and times out or fails correlation while leaving processes running. Very short-lived intermediaries may escape observation; a window created by a preexisting broker is not proven to belong to this launch. It must never be guessed by title or foreground coincidence. [Launch implementation](../../crates/porthole-adapter-windows/src/native.rs), [launch protocol](../../crates/porthole-protocol/src/launches.rs).

Consequently, `wt -w -1` is only a candidate, not an accepted recipe. Microsoft documents that it requests a new window, but this does not prove the resulting owning PID is in the adapter's observed tree. Windows Terminal also exposes environment inheritance/reload behavior, so token arrival must be tested independently of window creation. [Terminal arguments](https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments), [Microsoft's process/session design](https://github.com/microsoft/terminal/blob/main/doc/specs/%235000%20-%20Process%20Model%202.0/%234472%20-%20Windows%20Terminal%20Session%20Management.md).

Current screenshot support is bounded `PrintWindow` plus GDI-to-PNG conversion, with a three-second response timeout and one capture worker. GPU content may be missing. Pointer input, placement and continuous Windows capture are unsupported. None of this establishes Jackstay D3D support. [Adapter](../../crates/porthole-adapter-windows/src/native.rs), [Windows guide](../windows-desktop.md).

The [gouda acceptance report](../2026-09-07-windows-117-evidence.md) records real fixture launch, Unicode input, screenshot, authorization failures/approvals and cleanup on September 7, plus passing workspace gates in its September 8 integration follow-up. It tested another host, OS build and source revision; its PNG is historical evidence, not Beaufort evidence or proof of a terminal/Cleat chain.

## Startup candidates and lifetime

| Mechanism | Fit and constraints |
| --- | --- |
| Per-user Task Scheduler logon task | Preferred candidate: user-specific logon trigger, `Interactive` principal, limited run level, absolute executable/script paths and working directory. Microsoft specifies that `TASK_LOGON_INTERACTIVE_TOKEN` uses an already logged-on interactive session. Password/S4U/service logons do not satisfy this desktop requirement. [Logon type](https://learn.microsoft.com/en-us/windows/win32/api/taskschd/ne-taskschd-task_logon_type), [logon trigger](https://learn.microsoft.com/en-us/windows/win32/taskschd/logontrigger) |
| HKCU Run or per-user Startup shortcut | Simpler per-user bootstrap, but startup order is indeterminate and Windows may delay execution. A wrapper still needs readiness checks and duplicate prevention. RunOnce is inappropriate for every-login startup. [Run/RunOnce and Startup behavior](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys) |
| Windows service / SSH-created daemon | A service-account task is not the existing GUI login. An SSH transport may reach the in-session Cleat, but creating the desktop-owning process through an unrelated session is not equivalent. [Logon types](https://learn.microsoft.com/en-us/windows/win32/api/taskschd/ne-taskschd-task_logon_type), [adapter session checks](../../crates/porthole-adapter-windows/src/native.rs) |

A scheduled-task implementation should have one stable, owned task name, inspect any existing task before updating it, use registration/update under that name and remove only that task with `Unregister-ScheduledTask`. Registration/removal must be independently repeatable and must not imply stopping unrelated processes. [Registration API](https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/register-scheduledtask), [removal API](https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/unregister-scheduledtask).

Explicitly set indefinite runtime (`PT0S`), because the default task limit is 72 hours. Select `IgnoreNew`, not `StopExisting`, and keep the supervised action alive: IgnoreNew only covers a running task, so a wrapper that immediately detaches children still needs its own endpoint/process ownership check. Review battery and idle conditions so defaults cannot unexpectedly stop a persistent session. These are proposed settings, not installed configuration. [Execution limit](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-executiontimelimit), [instance policy](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-multipleinstances), [settings](https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/new-scheduledtasksettingsset).

The user has selected: preserve the same agent and terminal through detach, RDP disconnect and desktop lock; desktop requests may wait or fail clearly. Do not turn unlock or reconnect into an unconditional restart trigger. Windows session-ending policy is a separate threat to that lifetime: disconnected-session limits can eventually log the user off. Inspect effective policy before acceptance and record it; no policy was changed here. Sign-out/reboot cannot be promised to preserve a process. [Microsoft client policy reference](https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-admx-terminalserver).

## Token provisioning and isolation

Porthole accepts bearer identities and distinct launch/observe/drive/manage grants. Its existing operator CLI creates identities and approves pending requests; current policy is a development local-trust boundary, not protection from another process running as the same user. Policy data is `%LOCALAPPDATA%\Porthole\agent-policy.sqlite`. A launcher must use these supported operations rather than edit SQLite or introduce another authority model. [Permissions guide](../../README.md#agent-permissions), [store](../../crates/portholed/src/agent_store.rs), [authority ADR](../adr/0006-agent-permission-authority-deferred.md).

Pass token material through a captured process environment or equivalent private launcher handoff, never task arguments, command-line `--agent-token`, task XML, transcripts or evidence. Porthole's launch protocol supports an environment map, but a brokered terminal may not propagate it as expected. Approving a pending request does not execute it; provisioning must stage the necessary requests and retry after approval. Surface grants cannot refer to a window that does not yet exist. [Launch protocol](../../crates/porthole-protocol/src/launches.rs), [approval semantics](../../README.md#approval-inbox).

For startup across logins, choose explicitly between fresh per-run identity creation/revocation and a retained identity whose secret is protected for the Windows user. User-scoped DPAPI is an available Windows primitive for the latter; it ordinarily binds decryption to the same user and machine, but it does not create isolation from that user. This report recommends no new credential implementation until the startup decision chooses the lifecycle. [CryptProtectData](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata).

The production Porthole pipe currently derives from `USERNAME`; `PORTHOLE_RUNTIME_DIR` does not rename the Windows pipe, and the default policy database is shared per user. Its source explicitly notes pending multi-user DACL validation. The smoke refuses to coexist with an existing `portholed`. Do not assume a temporary directory makes an isolated daemon. The separately named daemon requirement applies especially to Cleat so RAD is untouched; any parallel Porthole need requires a deliberate endpoint/state solution. [Transport](../../crates/porthole-transport/src/lib.rs), [store](../../crates/portholed/src/agent_store.rs), [smoke](../../scripts/manual-windows-smoke.ps1).

## First executable packet and later acceptance

Proposed first packet: **Establish a reproducible Beaufort build environment and native desktop baseline**. Resolve Rust/MSVC environment discovery or installation under the implementation task, record exact versions and source revisions, run the four repository gates plus fixture build, and run the existing Windows smoke from the unlocked Session 1 only after checking no Porthole daemon conflicts. This packet needs no terminal startup architecture to demonstrate the local adapter. Source commands and required gates: [AGENTS.md](../../AGENTS.md), [Windows guide](../windows-desktop.md).

The user selected WinGet as the Rustup provisioning route. `winget.exe` resolves in the user's WindowsApps directory and reports `v1.29.290`. Microsoft's package repository contains `Rustlang.Rustup`, with native x64 installer URLs on `static.rust-lang.org` and silent installation support. The inspected `1.28.2` manifest establishes the package identity, not the latest available version. Proposed implementation commands are `winget show --id Rustlang.Rustup --exact --source winget`, then `winget install --id Rustlang.Rustup --exact --source winget`; record the resolved version before installation. Neither command was run during this research. [Package manifest](https://github.com/microsoft/winget-pkgs/blob/master/manifests/r/Rustlang/Rustup/1.28.2/Rustlang.Rustup.installer.yaml).

Rustup normally installs tools under `%USERPROFILE%\.cargo\bin` and attempts to update PATH, but existing processes can retain stale environments. Open a fresh GUI-session shell or use explicitly verified absolute tool paths; do not restart the existing RAD Cleat merely to refresh PATH. Verify `rustc -Vv`, `cargo -V`, the MSVC target and pinned formatter availability before building. [Official installation and PATH guidance](https://rust-lang.org/tools/install/).

Its acceptance evidence must include the real PNG visibly showing entered text, hash, sanitized commands/results, actual process/session identities, negative token/grant checks, and cleanup of only test-owned windows, identity and daemon. Record the existing RAD Cleat PID/path/session before and after. No compile-only success substitutes for that PNG. The stock smoke writes per-user policy records and must be treated as live validation, not a read-only probe. [Smoke](../../scripts/manual-windows-smoke.ps1).

Subsequent workflow acceptance must establish a named independent Cleat daemon, valid coding-agent authentication, the exact Porthole-launched terminal process tree and token arrival, remote attachment from kiwi, unchanged daemon/session/agent process identities and conversation marker through detach/reattach, lock and RDP disconnect, clear desktop unavailability responses, and resumed native screenshot after unlock. Startup registration twice must preserve the running session; unregistering must remove only the owned registration. Do these disruptive checks with the operator at the intended validation point. [Existing workflow](https://github.com/flotilla-org/porthole/issues/118).

Remaining decisions: select startup ownership/supervision and credential lifecycle; choose and prove the terminal executable/launch contract; choose the Cleat transport from its separate research; define bounded waiting versus immediate failure for unavailable desktop actions. The existing workflow's historical gouda-first wording should be aligned to the user's Beaufort-first scope. Jackstay graphics remains a separate research branch. This report does not resolve those choices or claim implementation completion; no build/test gates were run for this documentation-only research.
