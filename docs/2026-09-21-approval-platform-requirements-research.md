# Platform constraints for prepared combined approvals

Research date: 2026-09-21. Scope: the recording and debugging examples in the
[approval draft](2026-09-21-approval-model-draft.md), for
[Establish platform permission constraints for prepared combined approvals](https://github.com/flotilla-org/porthole/issues/165).
This is evidence for subsequent decisions, not an adopted contract.

**D** means platform documentation; **S** means source inspection/inference;
**L** means recorded live evidence; **U** means untested or unspecified.
Local source references describe Porthole commit `2e62cc1ec41f1b0d4d8cb5c7e3d159439017a5dc`.
No desktop probes, prompts, service changes or policy changes were performed.

## Requirement comparison

A successful check is an observation about a particular executor and session.
It neither guarantees later execution nor establishes the initiating agent's
permission. The matrix summarizes the evidence expanded below.

| Requirement | What preparation can establish | Effective scope and later target binding | Reuse, cancellation and remote completion |
|---|---|---|---|
| macOS app-level Screen Recording | S: daemon's `CGPreflightScreenCaptureAccess` result without calling its prompt path | D/S: app capture authority; a particular window is subsequently selected by the executor. Future-window selection remains our restriction. | D: user/management policy owns OS access. U: no exact grant expiry or active-stream revocation guarantee established. A signature from kiwi does not set this permission. |
| macOS Accessibility | D/S: trust of the checking process, without requesting a prompt | D/S: trusted accessibility client, not an individual agent/window grant. Protected discovery can itself require it. | S: Porthole reports restart required after granting. D: management can configure applicable app policy. U: immediate effects of revocation on every in-flight call. |
| macOS picker-mediated capture, alternative path | D: API availability/configuration; consent is obtained through the picker | D: selected content for the capture session; no separate broad recording grant. U: arbitrary future-window binding through this route | D: user can change/end sharing. U: no signed remote replacement of picker consent demonstrated. |
| Authorization Services/taskport | D: rights can be checked/requested with interaction disabled; requesting/preauthorizing can still acquire authority | D/S: stock debugging authorization is broader than RAD's motivating target. RAD requests it before the child exists. | D: documented ten-hour debugger session; target protections remain. L: signed remote completion works for our custom right only. U: taskport-specific beneficiary/revocation behavior. |
| Linux ScreenCast portal | D: interface version/source capabilities; selected-source readiness is unresolved until session negotiation | D: selected streams, belonging to a portal session. A future application window is not selected merely by approving its launch. | D: optional persistence; restore can prompt again and consumes a token. Closing a request/session differs from withdrawing persistent permission. |
| Linux polkit | D: noninteractive check can distinguish authorized/challenge; querying another user or supplying details can require privilege | D: action + subject; mechanism owns resource restrictions. Future target can be checked later by that mechanism. | D: standing rules or temporary authorization; retained action authorization can outlive target details. Authentication agent is session-associated, not a general remote-signature API. |
| Windows current Porthole | S: daemon checks its interactive input desktop; no capture-consent prompt path | S: `PrintWindow` screenshot of resolved HWND. U: future WGC recording behavior is not implemented here | S: locked/wrong desktop is a blocker. D: screenshot call depends on target rendering; no revocable capture grant is produced. |
| Windows WGC / UAC, future integration | D: WGC support/token state can be queried; neither is proof a later target operation succeeds | D: capture item selects content; elevation gives a process an administrator token, not one operation's authority | D: picker/capture and elevation are separate routes. U: no stock UAC mechanism accepting our remote signature established. |

## macOS: separate observation from acquisition

The adapter's `system_permissions()` calls `AXIsProcessTrusted()` and
`CGPreflightScreenCaptureAccess()`. Its `ensure_*_granted()` functions instead
try to trigger OS prompts on a miss. Thus reusing `ensure_*` in a supposedly
noninteractive planning pass would introduce interaction. The permission table
also distinguishes Accessibility restart behavior from Screen Recording.
These are **S** observations of [permissions.rs](../crates/porthole-adapter-macos/src/permissions.rs).
Apple documents the Accessibility options API as checking the current process;
requesting a prompt is optional, asynchronous and does not change that check's
return value. **D:** [AXIsProcessTrustedWithOptions](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions).

Discovery is not universally free. The screenshot path first checks recording
permission, then obtains geometry, propagating permission failures from that
step, before capturing through ScreenCaptureKit. A planner must preserve such
requirements instead of using protected metadata to build an allegedly inert
preview. **S:** [capture.rs](../crates/porthole-adapter-macos/src/capture.rs).
Neither API establishes a grant for a different process on another machine.

For provisioned machines, Apple's PPPC schema supports per-application
Accessibility policy. Its ScreenCapture entry explicitly says a profile can
deny capture but cannot grant it; `AllowStandardUserToSetSystemService` permits
a standard user to configure applicable privacy settings, not automatic consent.
**D:** [Apple device-management schema](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.TCC.configuration-profile-policy.yaml).
Schema changes for macOS 27 should not be silently applied to the macOS 26
machines used in our experiment. A custom Authorization Services mechanism is
not evidence that either TCC permission can be set by a signed remote decision.

A distinct documented route exists: `SCContentSharingPicker` grants access to
selected content for the capture session without requiring separate broad
screen-recording permission. The OS sharing control permits changing or ending
sharing. **D:** [Apple's privacy session](https://developer.apple.com/videos/play/wwdc2023/10053/).
Porthole's current permission checks do not implement that alternative. Its
presence means the common model should identify the acquisition route; a single
boolean named “screen permission” loses useful scope information. That is a
**proposed implication**, not a decision to replace unattended capture with a picker.

**U:** We have not established a public expiry timestamp for the app-level
recording grant, how promptly revocation stops each existing stream, or exact
capture/discovery behavior across lock, logout and fast user switching on our
machines. Report those as unknown; do not equate permission granted with an
available unlocked desktop. No new live TCC test was run.

## Debugging authorization

`AuthorizationCopyRights` without `interactionAllowed` reports when interaction
would be necessary. `extendRights` and `preAuthorize` are acquisition options,
not a promise of side-effect-free status. Apple also describes partial-right
results and cancellation. The planning adapter therefore needs an explicit
answer about whether its selected check merely observes or obtains authority.
**D:** [AuthorizationCopyRights](https://developer.apple.com/documentation/security/authorizationcopyrights(_:_:_:_:_:)).

RAD's inspected launch path requests `system.privilege.taskport` before creating
the child; its authorization request carries no target. **S:**
[RAD source](https://github.com/rjwittams/raddebugger/blob/17f68eb5ad2e1b13cf8e10b72cc5599f7b12e1a3/src/mac/demon/mac_demon.c#L523).
Apple documents ten hours of debugger authorization after administrator
authentication, with target entitlement/protection constraints still applying.
**D:** [debugger entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.debugger).
That permits an honest combined review of “this debug run” and “broader OS
debugging authority”; it does not prove only this process benefits or that
ending the run withdraws the OS authority.

The existing **L** evidence confirms automatic policy and Touch ID-backed
signing on kiwi can complete a custom right on comte; replay was rejected.
It does not establish a taskport integration, original-requester authentication,
or revocation of an already returned task port.
[Prototype evidence](../apps/macos/AuthorizationPrototype/NOTES.md).
Published authd source further shows that ordinary environment hints are not a
sufficient requester-identity boundary, and replacing a user rule with mechanism
evaluation changes credential semantics. **S:** the linked source analysis in
[prior macOS research](2026-09-21-remote-macos-authorization-research.md#what-an-authorization-plug-in-can-do).
**U:** beneficiary sharing, cached-right destruction, lock/logout and target-handle
lifetime require a separate, authorized taskport experiment before product claims.

## Linux: session capabilities and policy checks

The public ScreenCast interface exposes version and source types before
selection. `Start` normally asks the user and returns selected streams; the
PipeWire remote exposes only those stream nodes. Persistence modes cover no
persistence, application lifetime or until revocation. Restore tokens are
single-use; missing targets or withdrawn permission cause normal prompting.
There is no public selector here for “the future main window of this launch.”
**D:** [ScreenCast specification](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).

`Request.Close` cancels an outstanding request; `Session.Close` ends a session,
and the portal can also close it. Neither should be presented as proof that
stored consent was erased. **D/inference:** [Request](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html),
[Session](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Session.html).
**U:** lock behavior and persistent-consent administration depend on the chosen
backend and need verification there.

Porthole's KDE path currently requests one window, persistence mode 2, caches
`restore_data`, and retries selected restoration failures without that cache.
`restore_data` is not the public ScreenCast `restore_token` option; do not claim
this code demonstrates portable restoration. **S:**
[screencast.rs](../crates/porthole-adapter-kwin/src/screencast.rs).
RemoteDesktop input separately creates its own session. **S:**
[remote_desktop.rs](../crates/porthole-adapter-kwin/src/remote_desktop.rs).
A local portal approval does not authenticate the remote human or initiating agent.

Polkit offers a noninteractive authorized/challenge result. Its API restricts
checks with supplied details to sufficiently privileged callers; cancellation,
temporary-authorization enumeration and revocation are separate operations.
**D:** [Authority API](https://polkit.pages.freedesktop.org/polkit/eggdbus-interface-org.freedesktop.PolicyKit1.Authority.html).
Policy distinguishes active, inactive and other sessions. `AUTH_*_KEEP` can
reuse authorization for the same action and subject even when request variables
change; target-sensitive policy must not assume those variables bound a retained
grant. SSH sessions can use a text authentication agent. **D:**
[polkit manual](https://polkit.pages.freedesktop.org/polkit/polkit.8.html).
A remote-signature bridge would require a trusted integration; registering a
UI alone does not supply authenticated authorization. **Inference:** retain
mechanism-enforced target restrictions and separate grant revocation from stopping
an operation already admitted.

## Windows: avoid assuming one consent mechanism

Current Porthole uses `PrintWindow`, validates surface identity, and rejects
Session 0 or a mismatch with the active input desktop. Its permission endpoint
asks for an unlocked GUI session and has no prompt implementation.
**S:** [native.rs](../crates/porthole-adapter-windows/src/native.rs).
`PrintWindow` is synchronous and asks the owning application to render, so
preflight cannot guarantee capture success. **D:**
[PrintWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-printwindow).

For future WGC work, `IsSupported` is a capability check. The system picker
returns a capture item reusable for multiple sessions; frames start through
`StartCapture`. **D:** [screen capture](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture).
Win32 also provides `CreateForWindow(HWND)` for a specific existing window;
therefore a mandatory-picker claim would be too broad. **D:**
[CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow).
Borderless capture has its own consent/capability requirements. **D:**
[RequestAccessAsync](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess.requestaccessasync).
**U:** supported routes, item invalidation and lock behavior need a Windows capture
experiment; an absent future HWND cannot yet be bound to a capture item.

UAC approves use of an administrator token by an application; children can
inherit authority. Default administrator consent and standard-user credential
prompts differ, and elevation normally uses the secure desktop. **D:**
[UAC behavior](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/how-it-works).
Reading token information needs a handle with suitable query access. **D:**
[GetTokenInformation](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-gettokeninformation).
**Inference:** later cancellation cannot be described as revoking one operation's
UAC approval from an already elevated process. A preinstalled privileged service
could enforce our narrow delegation, but its provisioning and trust are separate
requirements. No reviewed source establishes a stock UAC remote-signature route.

## Consequences to decide next

The evidence supports considering explicit fields for acquisition route,
beneficiary, session binding, observed readiness, interaction location, residual
authority, and supported cancellation/revocation. These are proposals for the
preparation and scope decisions, not a settled wire schema.

“Record the new window” can collect app trust and agent requirements before the
window exists. Later binding must still verify the launch/window relationship;
a platform picker may remain outstanding. “Debug this program on comte” can
collect agent delegation and broad debugging authorization together, while
leaving target protection checks until the process exists. Neither workflow can
promise that one combined human decision atomically satisfies every OS step.

Experiments should be ticketed only when a design depends on the answer: TCC
revocation/lock behavior; real taskport beneficiary, cache and handle lifetime;
KDE restoration/session termination; Windows capture routes and desktop changes.
The existing custom-right result is sufficient evidence for retaining remote
signed approval as a design option without performing those experiments now.
