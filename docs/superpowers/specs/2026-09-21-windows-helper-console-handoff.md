# Windows helper and console handoff

Status: proposed implementation design. Operator direction establishes a native
Windows helper, modern Windows UX, manual login as sufficient, and an explicit
RDP-to-console action. Implementation details below remain reviewable proposals.
Decision: [Windows helper console handoff](https://github.com/flotilla-org/porthole/issues/173).

## Outcome and scope

A logged-in Windows host can keep running Porthole desktop automation after its
operator leaves RDP. The helper offers **Disconnect RDP — keep automation
running**. Explain alongside the action: **You stay signed in and the local
desktop stays unlocked.** This is console handoff, not logout or an unlock action.

First release uses UAC for each handoff and a short-lived worker. No installed
privileged service, automatic login, arbitrary elevated commands, or automatic
dismissal of unrelated UI. Porthole, Cleat and the agent keep their existing
lifetimes. Wheelhouse is not required. Jackstay Windows capture needs separate
implementation; one-shot Porthole screenshot evidence is not streaming support.

## Native Windows experience

Use **WinUI 3 and the Windows App SDK**, Microsoft's recommended foundation for
new native Windows applications. Operator agreed on 2026-09-22 to C# for the small
UI and control-plane client, with Rust/Win32 for the narrow privileged worker. The
PowerShell/WinForms prototype supplies evidence, not the production UI foundation.

The packaging experiment is in `prototypes/windows-helper-package`: a
self-contained WinUI interface plus a separate Rust elevation probe. It performs
no handoff. Installer format, signing, and production IPC remain open.

The subsequent `prototype/windows-helper-handoff` branch extends that experiment
with authenticated IPC and the fixed console operation. Approved and declined UAC
acceptance pass on Beaufort, with retained workload identities after reconnect;
see `prototypes/windows-helper-package/HANDOFF.md` for evidence and limitations.
The installed prototype also passed focus, typing, text readback and screenshot
capture against its test-owned editor while RDP was disconnected and the same
session was active at the console. Its temporary agent identity was revoked and
the original workload retained its process identities after reconnect. The
prototype's `evidence/console-desktop-20260922` records the native run.

`apps/windows/PortholeHelper` carries the first application source slice on a
clean branch. It narrows the visible UI to handoff and retains the guarded
worker protocol. Its installed development build passed the same native
console focus/text/capture and reconnect test. A subsequent source update added
a notification-area icon and close-to-tray behavior. Native UI Automation
confirmed that its hidden-icons button reopens the same helper process and that
right-click Quit ends only the helper; accessibility and Explorer restart still
need acceptance.
This is an unsigned development build;
installer/signing, startup, accessibility checks and a production privilege
review remain open.

- The Windows notification-area helper is the counterpart of the macOS menu bar
  helper. Both own the user-facing host status and setup experience. Keep shared
  concepts and behavior aligned as they grow: daemon health, desktop permissions,
  pending agent requests and their review, diagnostics, and lifecycle actions.
  The OS shell, permission prompts, session handoff and startup mechanisms remain
  platform-specific. Share code only where it makes those contracts clearer.
- A notification-area icon opens a compact status window. Provide a Start menu
  entry too, so the app remains discoverable when Windows hides its tray icon.
  The source-tree window used for the first handoff acceptance preceded the
  tray update. Its shell behavior and accessibility remain release gates.
- Use native shell integration for the icon and keyboard-accessible context menu;
  closing a menu must finish before a foreground handoff starts.
- Status window: host/session status, Porthole connectivity, desktop availability,
  last handoff outcome, the handoff action, and a diagnostics link. Avoid exposing
  process IDs, pipe names or token details in the normal flow.
- Use standard WinUI controls, typography, spacing, icons and title-bar behaviour.
  Follow system light/dark and contrast themes, text scaling, per-monitor DPI and
  reduced-motion preferences. Materials such as Mica are optional enhancement,
  never a readability or state cue dependency.
- Support keyboard-only operation, visible focus, meaningful UI Automation names
  and Narrator announcements for status changes. Do not communicate status by
  colour alone. Test with Accessibility Insights and Narrator.
- Show the elevation shield on the action. UAC provides elevation consent; no
  redundant confirmation dialog in the ordinary path. Do not imitate a UAC prompt.
- During the attempt, disable duplicate handoff requests and show progress.
  Closing the helper window normally returns to the tray. **Quit helper** ends
  only the helper; it must not terminate Porthole, Cleat or agents.
- Persist the last outcome for the next reconnect. Use an inline InfoBar/status
  for actionable failures; do not depend on a toast being visible over RDP, or
  send success notifications for routine background checks.

## States and user outcomes

| Condition | Behaviour |
| --- | --- |
| Active RDP session, matching live Porthole, desktop available | Enable handoff |
| Already at console | Show “Already running without RDP”; no elevation |
| Locked/unavailable desktop, missing or different-session daemon | Explain unmet prerequisite; no worker |
| User declines UAC | Cancelled; keep RDP connected, no handoff |
| Worker fails or identity changes before commit | Fail without signalling transfer; retain diagnostics |
| Foreground acquisition/grant fails | Do not commit transfer; offer explicit retry |
| Transfer succeeds, readiness succeeds | “Disconnected from RDP; desktop available” |
| Transfer succeeds, readiness fails or times out | “Transferred; desktop needs attention”; preserve session |
| Worker connection disappears around transfer | Record outcome unknown; inspect session before any retry |

Desktop availability and ability to activate a particular window are distinct.
Windows Search blocked activation in our tests while capture remained usable.
Do not silently send Escape, change foreground policy, kill shell UI, or retry
text input whose delivery is uncertain. A later explicit recovery action can be
designed separately. No promise that a readiness check guarantees every future
window operation.

## Handoff sequence

1. Unprivileged helper discovers its own user/logon/session and checks that the
   current session is RDP-active, with the expected same-session Porthole process.
2. Close only the helper's own menu. Start the installed worker through UAC.
3. Establish a bounded local rendezvous. Validate both peers and the retained
   Porthole process identity. The elevated worker reports **armed** and waits;
   elevation alone never initiates transfer.
4. After UAC, activate the helper's own normal window and verify actual foreground.
   Call `AllowSetForegroundWindow` for the validated Porthole PID only.
5. Commit once over the established channel. Worker revalidates its own session
   and peer, invokes the system `tscon` for that session with `/dest:console`,
   reports the result and exits. No shell or caller-supplied command is accepted.
6. Helper observes console-active state and probes Porthole availability. Record
   the transfer and readiness results separately; do not reconnect/log out/restart
   anything as an automatic recovery action.

The worker has a bounded arming lifetime (30 seconds initially), and only one
commit is accepted. Channel closure or expiry before commit aborts. After commit,
an uncertain outcome must be reconciled, not treated as permission to retry.
UI work stays responsive throughout; use asynchronous operations, not the blocking
timer callback from the throwaway prototype.

## Privilege and packaging boundary

Ship a dedicated worker with a fixed operation surface. Never elevate mutable
PowerShell source, resolve executables via PATH, accept arbitrary session IDs,
or allow the elevated process to write caller-selected files. Worker results go
over IPC; the unprivileged helper writes its own bounded local diagnostics.

Install executable code and dependencies in a location ordinary users cannot
modify. Establish publisher/signing and update rules before shipping. Pin the
Windows App SDK/.NET versions and verify deployment on a clean host. Choose the
package format after an installation/elevation spike proves the worker can run
from its protected location; MSIX capabilities and an unpackaged WinUI app with
a conventional installer are packaging options, not different UI designs.

Replace the prototype's named event plus mutable JSON with a local named-pipe
exchange using explicit access control, rejection of remote clients, and bounded
messages. Validate kernel-reported peer process identity, retained process handles,
image provenance, user/logon/session and operation lifetime. ACL membership or a
caller-supplied PID/nonce alone is insufficient authentication. Do not assume that
UAC authorizes later arbitrary messages from the same user.

First release supports same-user elevation. Different administrator credentials
must fail clearly unless a separately reviewed caller/session binding supports
them; never hand off the administrator's session by accident. This does not add
a general agent approval authority; preserve ADR-0006's boundary. A production
privilege review and adversarial IPC tests are release gates.

## Startup integration and ownership

Retain the GUI-session daemon registration from the
[startup work](https://github.com/flotilla-org/porthole/pull/162). The helper may
start at user login through an explicit setting, but a running helper is not
required to keep the daemon or agent alive. Reuse a matching daemon; expose
mismatch/multiple-instance failures. No implicit new coding agent.

Place UI sources under `apps/windows/PortholeHelper`. Keep the privileged operation
and its validation separate from presentation. The worker must not depend on UI
state or the existing development agent-token mint/grant shortcut. Acceptance
fixtures may use the current authorized development route and must say so.

## Delivery and verification

1. Establish WinUI build/packaging and protected worker deployment. Verify UAC
   publisher identity, clean installation/removal and dependency loading.
2. Implement tray/status UI and the handoff state machine with an injectable
   platform boundary. Test cancellation, duplication, expiry, worker death,
   daemon replacement and unknown post-commit outcomes.
3. Implement bounded, authenticated worker IPC and fixed console operation. Test
   wrong-session callers, stale identity, replay/duplicate commit, substituted
   executables, arbitrary path/command attempts and channel loss.
4. Native acceptance on Beaufort: approve and cancel UAC, hand off repeatedly,
   verify unchanged workload identities, and alternate focus/input/capture across
   test-owned windows without a viewer. Preserve the Search-blocked case as a
   clear failure, not a reason to dismiss user UI silently.
5. Validate keyboard/Narrator/contrast, DPI/text scaling, tray overflow, Explorer
   restart, notification suppression and helper quit without workload termination.

Run the repository's four Rust gates and add the Windows UI build/tests to CI.
Keep native session transitions and elevation tests explicitly operator-coordinated.
Cold login/startup acceptance remains separate; this design does not close it.

## Evidence and primary guidance

- [Prototype, approval/cancellation and console results](https://github.com/flotilla-org/porthole/tree/8eb8db7/prototypes/windows-helper-foreground).
  The basic sequence works; the grant's necessity and broad reliability are not proved.
- [Beaufort transition and Search diagnosis](../../2026-09-21-beaufort-login-startup.md).
- [Windows development path](https://learn.microsoft.com/en-us/windows/apps/get-started/),
  [app best practices](https://learn.microsoft.com/en-us/windows/apps/get-started/best-practices),
  [accessibility](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/accessibility),
  [notification area](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area).
- [Foreground grant contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow).
  Later user input can invalidate eligibility; an accepted grant is not permanent.
- [Named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)
  and [kernel client PID](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeclientprocessid).
  These support the IPC primitives, not proof that the proposed protocol is secure.
