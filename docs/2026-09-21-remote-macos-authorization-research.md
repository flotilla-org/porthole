# Remote macOS approvals: Authorization Services and alternatives

Research date: 2026-09-21. This extends [issue #128](https://github.com/flotilla-org/porthole/issues/128). It is a design investigation, not a decision to change system authorization policy. At the time of this research, no connection to comte, plug-in installation or authorization-database write had been performed. Subsequent custom-right installation and signed remote approval succeeded; see [prototype evidence](../apps/macos/AuthorizationPrototype/NOTES.md). The broader system-right questions below remain open.

## General direction

The question is how a human can approve requests on another machine. RAD taskport is the current concrete test, not the scope of the eventual facility. The most promising product shape is a common request/decision interface backed by platform-specific executors. An Authorization Services plug-in would be one possible adapter.

| Request family | Candidate integration | What the common interface must preserve |
|---|---|---|
| Operations our tools own | A local executor verifies authenticated remote approval before acting. | The exact operation and arguments, verified requester, host, expiry and completion result. |
| Authorization Services rights | Original prompt relay, or a mechanism explicitly configured for a selected right. | The actual right and its policy, including credential reuse and authorization lifetime. |
| Privacy consent such as Accessibility or Screen Recording | The OS's supported consent flow; managed policy where that service permits it. | Per-service restrictions and whether approval applies to an application identity rather than one action. |
| Desktop login/unlock or FileVault unlock | The corresponding remote administration or supported unlock facility. | A distinct session/host transition, not implied consent for a waiting application operation. |

This is a proposed taxonomy derived from the mechanisms examined below, not an implementation commitment. A common inbox should report when human interaction with the original OS UI is required, rather than presenting an Approve action that cannot complete the underlying request. Operations outside these families need their own investigation; there is no evidence here for a universal macOS approval hook.

For a request our code owns, the host can register the operation directly with the broker. For an unrelated application's OS prompt, discovering the pending request, identifying its real requester, obtaining approval and correlating the result are separate integration problems. Plug-ins only see configured Authorization Services invocations, while prompt capture provides UI evidence rather than a structured, authenticated operation description.

The important design distinction is between the requested action and the scope actually granted. Preserve both in the UI and audit record. Approval of desktop control, approval of a debugging authorization period, persistent privacy consent and approval of one target operation are different grants.

## First concrete experiment

For RAD on comte, first establish whether a human can approve its existing prompt through Apple's Screen Sharing in the same graphical session. That gives us a baseline for later Porthole/Jackstay prompt relay. In parallel with future design work, a custom test right can establish the authorization plug-in lifecycle without changing a system right. Neither experiment alone proves the general approval model.

## What RAD requests

The inspected RAD checkout is commit `17f68eb5ad2e1b13cf8e10b72cc5599f7b12e1a3`. In `src/mac/demon/mac_demon.c`, `mac_dmn_taskport_authorization_status` at line 523 creates an Authorization Services reference and requests `system.privilege.taskport` with ExtendRights, PreAuthorize and optional InteractionAllowed. It supplies no environment or target metadata. `dmn_ctrl_launch`, line 3274, calls it before launching the child. The actual `task_for_pid` call follows process creation. The debugger's entitlement file declares `com.apple.security.cs.debugger`. [RAD source](https://github.com/rjwittams/raddebugger/blob/17f68eb5ad2e1b13cf8e10b72cc5599f7b12e1a3/src/mac/demon/mac_demon.c#L523), [entitlements](https://github.com/rjwittams/raddebugger/blob/17f68eb5ad2e1b13cf8e10b72cc5599f7b12e1a3/src/mac/raddbg_debug.entitlements).

Apple documents an administrator prompt and a ten-hour authorization period for a non-root debugging tool. The debugger entitlement does not override target protection: target signing/get-task-allow and system protections still matter. [Apple debugger entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.debugger).

Read-only inspection on **kiwi**, macOS 26.6 build 25G72, showed the following taskport rule. This is not an observation of comte's current configuration:

| Field | Observed value |
|---|---|
| class | user |
| group | _developer |
| authenticate-user | true |
| shared | true |
| timeout | 36000 seconds |
| allow-root / session-owner | false / false |

The rule's own comment limits this authorization to same-user debugger/target access and advises administrators not to modify the right. Evidence: `security authorizationdb read system.privilege.taskport`; the policy also appears in [Apple's authorization database source](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/authorization.plist).

The installed `taskgated(8)` manual identifies taskport as the right used for callers with the public debugger entitlement, and describes preliminary kernel access checks. Apple's published kernel also performs POSIX and mandatory-access checks before its task-access-server call. A plug-in approving an Authorization Services right therefore cannot waive every task-port restriction. [XNU task_for_pid implementation](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_proc.c#L5695).

Inference: approving RAD's current preauthorization could unblock its launch, but cannot itself mean “only this executable/PID.” The request precedes that PID and grants a general right. We would need cooperation from RAD or an executor that enforces the narrower operation.

## What an authorization plug-in can do

Apple documents plug-in bundles in `/Library/Security/SecurityAgentPlugins` and mechanisms selected by authorization policy. They are invoked for configured rights, rather than acting as global observers of every permission dialog. The privileged host runs as root and cannot use WindowServer; the unprivileged host can present UI. Some policy configuration uses `AuthorizationTagsPriv.h`, explicitly outside the public API. [Apple plug-in integration guide](https://developer.apple.com/documentation/security/extending-authorization-services-with-plug-ins).

The invocation may complete asynchronously through `SetResult`. A small mechanism could ask a local broker to obtain a remote decision, then return allow, deny or cancellation. This is a proposed architecture supported by the lifecycle, not an Apple-provided remote-approval service. Deactivation and destruction must cancel outstanding work. [SetResult](https://developer.apple.com/documentation/security/authorizationcallbacks/setresult), [public lifecycle header](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/libsecurity_authorization/lib/AuthorizationPlugin.h#L289-L326).

Two source-level complications matter. First, a `user` rule can succeed using cached credentials before mechanisms run; fresh authentication still needs a valid user credential. Replacing it with `evaluate-mechanisms` changes those semantics. Second, ordinary requester hints are not necessarily trusted: authd initializes client/creator hints, then copies caller environment into the same mutable dictionary. A supplied PID or ordinary audit-token hint is insufficient as an identity boundary. These are observations of published Security source, not validation of the binary on comte. [User-rule path](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/engine.m#L936-L1035), [mechanism-rule path](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/engine.m#L1100-L1164), [environment/hints](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/engine.m#L1618-L1620), [dictionary copy](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/authitems.c#L615-L627).

Thus “replace the password prompt with a remote Approve button” is plausible for a chosen policy, but not a drop-in promise of equivalent scope. A custom right would validate our mechanism without granting `system.privilege.taskport`. Integrating the latter would require an explicit policy decision and tests of requester identity, caching and session behavior.

The API remains documented. Apple DTS nevertheless recommends avoiding plug-ins where possible in a login-integration discussion because of fragility. A separate DTS correction says **specific versioned rights**, not necessarily the whole database, can reset during upgrades. Neither Platform SSO nor a screen-unlock plug-in is established here as a substitute for taskport approval. [DTS advice](https://developer.apple.com/forums/thread/764756), [DTS corrected upgrade explanation](https://developer.apple.com/forums/thread/841012).

## Alternatives and their scope

| Approach | What it gives us | Boundary / fit |
|---|---|---|
| Apple Screen Sharing / Remote Desktop | A human controls the existing remote desktop and responds to the original prompt. | Best first test for RAD. Use the active display containing RAD, not another user's virtual desktop. Documentation does not guarantee every security prompt accepts remote input. |
| Porthole + Jackstay prompt relay | The original UI becomes part of our remote operator workflow. | Needs evidence that the relevant prompt can be captured and accepts human-controlled input. Ordinary window-capture success does not prove this. |
| Local broker with remote approval | A request describing host, requester and operation is approved elsewhere and enforced locally. | Strong fit for operations we own. It cannot grant arbitrary macOS rights just because our approval service says yes. |
| Authorization Services plug-in | A selected authorization policy can wait for our broker's decision. | Useful if transparent integration with an existing right is essential. Custom-right proof first; system-right policy changes remain a separate decision. |
| Developer-machine provisioning | Removes some recurring debugger prompts under an administrator-chosen policy. | Broader standing authorization, not approval of each request; documented DevToolsSecurity scope is Apple-signed tools. |
| MDM / PPPC | Deploys supported per-service privacy policy to managed hosts. | Useful onboarding infrastructure, not a universal approval queue or taskport authorization mechanism. |

Apple documents remote control and the distinction between sharing an active display and using a virtual display. The exact RAD prompt remains a live test. [Screen Sharing](https://support.apple.com/guide/mac-help/mh11848/mac), [ARD display choice](https://support.apple.com/guide/remote-desktop/apd4f46319e/mac).

For our own remoting, Apple DTS describes a network/global daemon plus GUI agents, with Aqua and LoginWindow agents for pre-login support. DTS reports ScreenCaptureKit working in that architecture on macOS 14.4 and later. This supports investigating prompt relay in the correct session; it does not establish visibility or input acceptance for every SecurityAgent prompt. [DTS pre-login capture architecture](https://developer.apple.com/forums/thread/814152).

A broker is a proposed product design: the remote UI signs or otherwise authenticates a bounded decision; a local service verifies it and performs only the matching operation. Apple supports helpers, authenticated clients and app-specific authorization rights. Root does not remove mandatory-access restrictions, and a daemon's TCC failure does not produce a normal consent UI. We must separately satisfy the operating system's requirements. [DTS privilege guidance](https://developer.apple.com/forums/thread/708765).

`AuthorizationExternalForm` supports passing an existing authorization reference between processes. Apple's SDK header describes it as opaque, sensitive, and bounded by sessions/processes/time. It is not evidence for a portable approval credential minted on kiwi and redeemable on comte; our cross-host approval protocol would be separate. [Authorization header](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/libsecurity_authorization/lib/Authorization.h).

The installed Apple `DevToolsSecurity(8)` manual explicitly describes Apple-code-signed debugging/profiling tools and membership in `admin` or `_developer`. That is narrower than a documented fix for RAD's own caller. RAD's error text suggests the tool, but we should verify actual behavior before adopting provisioning as a solution. No policy enable/disable command was run.

PPPC can manage Accessibility for specified clients. `AllowStandardUserToSetSystemService` for ScreenCapture permits user approval; it is not silent approval of capture. The restricted persistent-content-capture entitlement is separately approved by Apple and does not promise authority over unrelated prompts. [PPPC settings](https://support.apple.com/guide/deployment/dep38df53c2a/web), [PPPC identity schema](https://developer.apple.com/documentation/devicemanagement/privacypreferencespolicycontrol/services-data.dictionary/identity), [persistent capture entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.persistent-content-capture).

## Session states must be tested separately

| State on comte | Research conclusion |
|---|---|
| Logged-in, unlocked desktop | Start here with the existing RAD instance and Apple's screen-sharing path. Record whether the authorization result reaches RAD and it obtains the task port. |
| Logged-in, locked desktop | Test whether an operation can complete while the desktop remains locked. Unlocking the desktop is a different grant. No such success is established here. |
| Logged-out LoginWindow | Requires the corresponding GUI-session architecture; working Aqua capture is insufficient evidence. |
| FileVault after restart | Separate from LoginWindow application approvals. Apple now documents SSH unlock on Apple silicon/macOS 26+ with Remote Login enabled and network connectivity. This is not an arbitrary permission-approval channel. |

The FileVault guidance was updated September 17, 2026. We should not carry forward an unconditional “FileVault requires physical presence” assumption. Comte's actual eligibility/configuration was not checked. [Apple FileVault guide](https://support.apple.com/guide/deployment/dep82064ec40/web).

SSH also creates a different session context. Merely matching the GUI user's UID, setting InteractionAllowed, or executing an authorization command through SSH does not prove that credentials will be shared with RAD's graphical session. Test session behavior rather than recommending an unverified shell command. [Apple TN2083](https://developer.apple.com/library/archive/technotes/tn2083/_index.html).

## Bounded next experiments

1. Read comte's OS build, debugger signature/entitlements and current taskport rule. Identify the actual RAD process and GUI session. Preserve these observations alongside the result.
2. Trigger one RAD launch and approve the genuine prompt manually through Apple's active-display sharing. Observe authorization completion, task-port acquisition and breakpoint arrival independently. Do not clear shared credentials on a machine running other debugging work merely to force a prompt; use an isolated account/session for repeatability if needed.
3. Repeat with Porthole/Jackstay human input, measuring prompt visibility and input acceptance separately. Keep approval credentials out of agent-visible transcripts and recordings.
4. If a native remote approval card is still desirable, prototype a custom right in an isolated macOS environment. Exercise allow, deny, expiry, cancellation, network loss and requester/operation binding. This proves the mechanism lifecycle, not taskport integration.
5. Decide whether the intended grant is a debugging authorization period or a single target operation. For the latter, design the broker/RAD enforcement path before changing a system right. Neither cached taskport credentials nor an acquired Mach task port should be treated as a revocable one-operation token without evidence.

This fits [ADR-0006](adr/0006-agent-permission-authority-deferred.md): Porthole can present requests and enforce scoped decisions; the eventual authority can serve Flotilla/Wheelhouse and other operations. Jackstay carries the remote surface/input. A macOS authorization plug-in would be one platform adapter, rather than the definition of the cross-platform approval model.

## Password alternatives: remote human approval, smart cards and automation

Follow-up, 2026-09-21: comte lacks a fingerprint reader. The desired outcome can be approval on another device or previously delegated authority for an automated host; displaying the same password prompt remotely is only one option.

### An operator authenticates on another device

A proposed mechanism can send a challenge to a broker, display the request on kiwi, and require local authentication before kiwi signs a bounded approval. Comte would be provisioned to trust that operator key for selected rights and would verify the challenge, host, right, expiry and decision. A private signing key on kiwi can be protected with Secure Enclave access controls requiring local authentication. This composes Apple's local key protection with a protocol we would implement; it is not a built-in cross-Mac Touch ID facility. [Apple Secure Enclave key protection](https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave).

The Authorization Services mechanism on comte can wait for that result through its asynchronous callback. Setup must explicitly grant the remote identity authority under the chosen policy. A successful fingerprint check on kiwi alone conveys no macOS rights on comte. The taskport rule-class and requester-binding questions above still apply. [Apple SetResult contract](https://developer.apple.com/documentation/security/authorizationcallbacks/setresult).

### Smart cards and YubiKeys

Apple explicitly lists **authorization dialogs** among supported smart-card authentication uses. Its PIV support and account pairing provide an existing credential-based route. A YubiKey with PIV functionality can participate in this path; the relevant mechanism is its smart-card certificate/private key, generally protected by a PIN, rather than merely having a FIDO/WebAuthn security key. Not every YubiKey model has PIV. This establishes a supported alternative credential class, not a verified result for RAD's exact dialog on comte. [Apple supported smart-card functions](https://support.apple.com/guide/deployment/depc47f60521/web), [Apple smart-card use/pairing](https://support.apple.com/guide/deployment/depc705651a9/web), [Yubico PIV documentation](https://docs.yubico.com/hardware/yubikey/yk-tech-manual/yk5-apps-piv.html).

A token plugged into kiwi is not automatically available to comte. For our proposed remote-approval protocol it could protect the operator's signing credential on kiwi; making it a native credential visible to comte is a different integration.

CryptoTokenKit is worth considering for the latter. Apple DTS describes an extension that exposes a certificate and private-key stub to local applications, forwards signing to hardware or a network service, and returns the signature. That is evidence for remote signing, not a guarantee that every authorization dialog can consume such a token. [DTS network-token workflow](https://developer.apple.com/forums/thread/766972).

Apple explicitly says persistent tokens are per-user and unsuitable for validating login because they become available only after login. Its separate authentication guide describes smart-card extensions, system-owned UI and registration with SecurityAgent. Login and keychain unlock also require different cryptographic operations. We should test the intended SecurityAgent context before proposing a network-backed token as a password replacement. [CryptoTokenKit overview](https://developer.apple.com/documentation/cryptotokenkit), [token authentication guide](https://developer.apple.com/documentation/cryptotokenkit/authenticating-users-with-a-cryptographic-token).

### An automated host uses delegated policy

Proposed design: an operator configures a standing grant for specified rights and verified workloads/accounts. The local broker checks that policy and returns a decision without human interaction; operations outside the grant can still request remote approval. This is machine authorization under prior delegation, not a claim that a human approved each request. The common audit record should distinguish these origins.

For operations we own, that can be implemented at the executor. Existing macOS rights need an appropriate supported policy or mechanism integration and may retain broader caching semantics. TCC, target code-signing restrictions and encrypted-key access remain separate. A cryptographic token requiring touch/PIN on every use would itself remain an interaction requirement; an unattended credential needs a deliberately different policy.

Recommended comparison for a later prototype: (1) a custom right completed by a Touch ID-protected signed approval from kiwi; (2) the same right completed by a narrowly provisioned standing policy; (3) native PIV authorization as a reference authentication flow. Defer adoption of network CryptoTokenKit for SecurityAgent until its token visibility and interaction requirements are demonstrated.
