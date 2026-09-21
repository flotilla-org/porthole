# Cross-host approval model: working draft

Status: proposed for discussion, 2026-09-21. This follows the successful
[custom-right experiment](../apps/macos/AuthorizationPrototype/NOTES.md).
It does not adopt an authority service, change taskport policy, or supersede
[ADR-0006](adr/0006-agent-permission-authority-deferred.md).

User direction: agent approvals and system approvals belong in one model.
Combined approval of explicitly described agent and system effects is accepted.
The current discovery of new permissions partway through an operation, such as
recording a new window, should be addressed as part of this design.
Bounded future targets are accepted: a target satisfying the approved selection
rule can be bound when it appears without another approval. Notifications,
the TUI and native helpers should be considered together as approval surfaces.
Approval requirements are policy-controlled: standing delegation, explicit
confirmation through an authenticated operator session, or fresh authentication.
The executor's trusted policy sets the minimum; the requester cannot lower it.

## What the experiment establishes

An Authorization Services mechanism on comte completed a custom right after
either a standing-policy decision or a Secure Enclave signature produced on
kiwi following Touch ID. Replaying the signed response was rejected. This proves
the integration path; the custom right grants no privileged operation.

The broker recognizes its root mechanism peer, not the original application.
It has no verified workload identity or operation-specific enforcement. Its
hostname is a label, not an enrolled host identity. The experiment used SSH
transport; it did not implement general host enrollment or a trusted approval UI.

## Proposed terms

An **approval request** asks an authorized decision maker to permit a described
scope. It includes the reason the request arose, but that reason is not itself
an enforceable restriction.

An **effective scope** states the authority that the executor or operating
system will actually make available: to whom, over which resources, for which
operations, for how long, and with what cancellation or revocation behavior.

An **approval decision** records allow or deny against one immutable request
revision. Its origin is either a human decision or a named standing policy.

A **grant** is permission available for use following an accepted decision.
A decision may instead complete an already-waiting OS authorization call;
that call's success does not establish that the motivating operation succeeded.

An **executor** performs or admits the operation and checks the restrictions
it claims to enforce. An OS adapter reports the OS's actual scope and limits.
An **approval surface** presents the request and collects the decision.
Flotilla or Wheelhouse can host that surface without owning every executor.

## Request contents

| Part | Meaning |
|---|---|
| Destination | Enrolled host identity, enforcing service and service incarnation; hostname separately for display. |
| Initiator | The authenticated agent or person that started the workflow, if known; preserve the chain when an application acts for another principal. |
| Direct requester | The peer the local executor actually authenticated, with identity evidence. An asserted app name, PID or bundle ID is not verification. |
| Beneficiary | The account, process, application or service that receives authority. This can differ from the initiator. |
| Intent | The motivating operation and target, with provenance for supplied descriptions. |
| Effective scope | Operations, resource selection, beneficiary, lifetime, use limit, enforcement owner and revocation limits. |
| Binding | Request ID, revision, unpredictable challenge and digest of the complete approval-relevant contents. |
| Decision authority | Which enrolled operator keys or standing policies may decide this scope. |

The host authenticates the request to the approval surface. The surface signs
the request binding, effective scope and decision for the destination. The
executor verifies both the signature and the signer's delegated authority.
An enrolled key is not automatically authorized to approve every right.

Descriptions can explain a request but cannot add restrictions the executor
does not enforce. Changes to scope, beneficiary or bound operation require a
new revision and invalidate outstanding decisions. Unknown requester identity
must remain unknown in both policy evaluation and the UI.

## Two forms of approval

These are forms of scope within the common model, not separate agent and system
approval tracks. Proposed direction, pending discussion: support both
exact-operation approval and broader capability or OS-right approval. Show
effective scope prominently and never silently widen a narrow request to fit a
platform's capabilities.

For an exact operation, the executor holds the immutable operation while the
human decides. It revalidates the target before execution. A reused PID, replaced
executable or changed arguments invalidate the request unless the approved
scope explicitly permits that change. Which resources must be pinned is
operation-specific; a path or a file hash alone does not describe every effect
of launching a program.

For a capability grant, the executor checks each admitted operation against
the scope. A grant can cover a surface, a workflow or a time interval. The UI
must distinguish the first operation that prompted the request from later
operations the grant also permits. Persistent delegation is an explicit policy
choice, not a side effect of pressing Approve.

For an OS right, the adapter describes the actual OS authorization and any
known reuse. If it cannot enforce a requested target or lifetime, it reports
that limitation before approval. Unsupported restrictions must not be accepted
as if they were enforced.

## One operation with several authorization requirements

An agent's proposed operation can require both delegated permission for the
agent and an OS authorization for its executor. These belong to the same
operation record. Each requirement identifies its beneficiary, effective scope,
enforcement owner, current satisfaction and supported approval route.

For example, an agent asks RAD on comte to debug one test program. The host
checks whether that agent may initiate the debug operation and whether the
debugger has the necessary OS authority. Either requirement can already be
satisfied while the other still needs a decision. Existing OS authority does
not by itself authorize every agent; an agent grant does not waive OS checks.

Agreed presentation: show the intended operation and all outstanding
requirements together. One human decision can authorize several explicitly
listed effects if the signer has authority for each and the adapters can carry
them out. Where the OS requires its own interaction, the same operation remains
pending with that requirement visible. A common workflow does not imply that
every platform supports one-click completion.

The decision must bind the proposed effects as well as the initiating operation.
Approving an agent to debug one program cannot silently authorize a broader
debugging period. If that broader grant is required, surface it before approval.
Scope expansion discovered afterward requires another decision.

Requirements can be shared without merging operation identity: several agent
requests might benefit from one already-approved OS permission. Each still has
its own agent authorization and outcome. If satisfying an OS requirement
succeeds but the operation later fails or is cancelled, record the residual OS
authority rather than claim the entire approval was rolled back.

When a narrow agent action requires a broader OS grant, offer explicit combined
approval. Completion still depends on each adapter satisfying its requirement.

## Discover requirements before execution

The proposed operation has a preparation phase that gathers all requirements
already derivable from its intent. Evaluate independent requirements together
and report every known unmet requirement, rather than returning only the first
permission error. Preparation neither consumes grants nor starts the requested
capture, input, process launch or other effect. Metadata discovery itself must
respect authorization; a planner cannot inspect protected resources merely
because it has not started the final operation.

The inspected recording path demonstrates the current sequencing:

- `routes/attach.rs` requires all-surfaces `Manage` for search and track.
- `routes/capture_sessions.rs::post_surface` requires `Observe` and `Record`
  against an existing surface before creating the session.
- The macOS capture implementation then checks Screen Recording.
- `routes/agent_guard.rs` consumes a one-use grant during authorization, before
  the later capture setup has succeeded.

These are source observations, not a reproduced live failure. The existing
capture endpoint already combines Observe and Record; the wider operation
crosses endpoint and platform boundaries that are checked separately.

For a proposed "record this window" operation, collect discovery/tracking,
agent capture access and the platform capture prerequisites into one plan.
Distinguish file-output readiness from authority unless an executor actually
enforces a file-output restriction. In the current CLI, duration and output
are held by the recording client; the daemon is asked to create a capture
session. The new model must not imply those client-side choices are already
enforced by the daemon.

Some targets only exist after an authorized earlier step. Proposed solution:
approve a constrained future target, then bind its concrete identity when it
exists. For example, the selected window belonging to a specific application
launch can become a SurfaceId without introducing a second grant, provided
the executor proves it satisfies the approved constraint. An app display name
alone is insufficient. Selecting among several matching windows also needs an
explicit rule; ambiguity cannot silently widen the selection.

Planning a program cannot predict every data-dependent permission need. Mark
unresolved requirements as unresolved. Where discovery itself requires an
effect, expose an authorized preparation stage and its consequences. A genuinely
new requirement pauses the operation at a safe boundary, updates the same
operation record and requests approval for the additional effects. Already
completed work is recorded; restarting the whole action is not an implicit retry.

At execution, revalidate the prepared target, grants and platform readiness.
Reserve or consume one-use authority only at the appropriate admission point,
with concurrency handled by the executor. A preflight is an observation, not
a guarantee that the OS or target cannot change afterward. Keep the requirement
description next to the implementation that enforces it so preparation and
execution do not acquire separate, drifting permission lists.

Agreed direction: support constrained future targets. For example, approval can
cover the main window of the application launched by this operation, with its
concrete identity bound afterward. The exact selection and binding rules remain
to be specified; neither an unrelated launch nor an ambiguous match inherits
the approval automatically.

## Approval surfaces and remote execution

The agreed policy direction separates approval strength from UI choice.
Standing delegation permits qualifying operations without a fresh human decision;
explicit confirmation uses an authenticated operator session; fresh authentication
adds a required local authentication step. For a combined request, satisfy the
strongest applicable requirement once where the requirements are compatible and
the decision binds every effect. Mandatory platform interactions remain required.
Authentication does not widen the approved scope. Creating a standing delegation
is itself an authorization operation whose required strength must be decided.

The operation and its authorization requirements outlive any particular UI.
Proposed roles:

| Component | Responsibility |
|---|---|
| Execution host | Prepare requirements, hold the operation, validate decisions, enforce scope and publish progress and results. |
| Approval client | Display the authenticated request and effective scopes, collect an explicit human decision and show its status. |
| Operator signer | Authenticate locally when required and sign the exact reviewed decision with an enrolled key. |
| Notification presenter | Announce a pending request and open its review view; notifications do not own request state. |

These roles need not be separate processes. A Swift helper on kiwi could host
both the approval client and signer. A helper on comte could serve local
onboarding or approval without becoming the required place for a remote human
to authenticate. Later platform equivalents can implement the same roles.
Hosting a UI does not itself grant the operator authority over the destination.

The current Swift helper implements onboarding and daemon status, not this
inbox. The existing inline Ratatui reviewer polls pending requests and grants
every two seconds. Proposed evolution: retain the compact request/grant views,
but let request details describe the whole operation, its requirements and
effective scopes. A shared snapshot/update interface should support all clients;
reconnecting clients resynchronize instead of relying on every event arriving.

Example proposed flow:

1. Comte prepares a recording operation and publishes its pending requirements.
2. An explicitly subscribed approval client on kiwi shows the request in the
   TUI or native inbox and, if configured, presents a notification.
3. The operator reviews the combined scope and chooses Approve. Local
   authentication follows that action; incoming requests do not themselves
   trigger Touch ID dialogs.
4. The signer signs the reviewed revision. Comte accepts it only if still valid,
   then satisfies the requirements and executes or reports the next blocker.
5. Every connected view receives the resulting decision and operation state.

The operator signer must bind its confirmation to the same operation and scope
the human reviews. A caller-supplied reason string plus an unrelated biometric
success is insufficient. If the TUI cannot provide a trusted review surface
under the chosen threat model, the native signer must present the scope itself.
This trust boundary needs design before exposing general signing to clients.

Running the TUI through SSH on comte must not copy kiwi's private key to comte.
That TUI could request completion through the enrolled approval client on kiwi,
or the operator can run the TUI locally against comte's endpoint. The method of
viewing a terminal is independent of which device signs a decision.

Proposed first notification behavior is "Review", opening the current request
revision in an inbox. Keep sensitive operation details out of notification
previews by default. Quick approval directly from a notification is a later
choice, dependent on presenting the full effective scope and authenticating
the decision where required. Native OS consent that cannot be completed remotely
remains a visible requirement with a supported local completion route.

Closing a TUI or dismissing a notification does not deny or cancel the operation.
Denial, cancellation and local notification dismissal are distinct actions.
No surface has an exclusive decision lock simply because it is open. Resolution
is atomic at the decision authority; stale revisions and conflicting decisions
receive the recorded result without another execution. Notification activation
refreshes state before offering an action, including when another device has
already resolved the request.

If a connection drops after signing, show delivery or confirmation as unknown
until reconciled with the execution host. Do not label a local signature as
execution success. Outstanding OS steps, residual grants and operation failure
remain visible after an approval decision has been accepted.

Notification destination, active-review suppression and escalation are still
open UX choices. Start from explicitly enrolled/subscribed approval clients;
do not assume every host in a fleet should notify for every request. These
choices should not affect whether an otherwise authorized decision is valid.

## Three lifetimes and separate outcomes

The response deadline limits when a signed decision can be accepted. The grant
lifetime limits future use of permission. The operation lifetime describes an
action already admitted, such as a recording or debugging session. They need
not end together.

Use separate records for request resolution, grant state and execution outcome.
For example, an approved request can lead to an expired unused grant, or to an
operation that fails because the OS rejects its target. A lost response after
execution begins can leave the outcome unknown; it must not trigger an automatic
second execution of a non-idempotent action.

For a single-use approval, redemption and cancellation need one atomic winner
at the executor. Duplicate submissions return the recorded status without a
second admission. Process exit, request expiry and broker restart must not leave
an old challenge redeemable for a new invocation. Persist consumption where
recovery requires it, or invalidate the old incarnation and require a new
decision. This is an at-most-once admission rule, not a promise of exactly-once
external effects across crashes.

Revoking a grant stops future uses at the enforcement points that check it.
Stopping an admitted operation needs its own supported cancellation mechanism.
Neither action implies reversal of completed effects or invalidation of an
OS handle already handed to an application.

## Applying it to RAD and taskport

At RAD commit `17f68eb5ad2e1b13cf8e10b72cc5599f7b12e1a3`,
`mac_dmn_taskport_authorization_status` requests the general taskport right
without a target. `dmn_ctrl_launch` calls it before creating the child process,
then calls `task_for_pid`. The attach path calls `task_for_pid` directly.
[RAD source](https://github.com/rjwittams/raddebugger/blob/17f68eb5ad2e1b13cf8e10b72cc5599f7b12e1a3/src/mac/demon/mac_demon.c#L523).

Apple documents a ten-hour authorization session for a non-root debugger after
administrator authentication, with target protection still applying.
That is documented stock behavior, not a promise that replacing the right's
mechanisms preserves it.
[Debugger entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.debugger).

A truthful approval card for the existing path could explain that RAD wants to
launch a named program, while stating that the permission being authorized is
a broader debugging right. It cannot claim that only that program becomes
debuggable, that only RAD can benefit, or that the grant is revoked when the
window closes without verifying those boundaries.

RAD cooperation could enforce an application workflow restriction. Stronger
operation-specific authority would require an executor that owns the relevant
privilege and target binding. If the original debugger can use ambient taskport
authority independently, an extra approval check in its workflow is not a
system-enforced boundary against that debugger. Handing it a task port likewise
does not establish an expiring or revocable debugging session.

## Next evidence to collect

1. Keep the custom right for a requester-identity experiment. Compare a direct
   client connection with the mechanism path, so the test distinguishes a
   verified requester from a verified intermediary. For direct macOS clients,
   investigate the documented XPC peer-signature checks. Apple notes that
   executable identity does not distinguish in-process plug-ins.
   [XPC identity APIs](https://developer.apple.com/forums/thread/681053?answerId=716586022),
   [client authorization limits](https://developer.apple.com/forums/thread/708765).
2. Exercise two requesters and two pending operations: substitution, replay,
   cancellation races, restart and lost replies. Confirm the OS call's result
   for signed deny and timeout as well as allow.
3. Pick one operation we own with meaningful target enforcement, and show its
   exact-operation approval beside the custom OS-right request. This tests
   whether the model works beyond taskport before building a shared inbox.
4. Separately decide whether broader taskport authorization is useful enough
   to justify a system-right integration. Verify requester, beneficiary,
   credential reuse and session behavior before proposing a concrete policy.

Combined approval and bounded future targets are agreed. Requirement preparation,
target binding details and the relationship between review surfaces and signers
need concrete design. Operator enrollment, delegated policy breadth and the first
owned operation can then be decided against examples. No generic policy language
or service extraction is needed for this draft.
