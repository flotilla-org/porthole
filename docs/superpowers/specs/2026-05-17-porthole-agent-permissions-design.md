# Porthole Agent-Permissions Design

Date: 2026-05-17
Status: approved

## 1. Purpose

Porthole already has a system-permissions layer: macOS decides whether the
Porthole bundle may use Accessibility and Screen Recording. Agent permissions
are a different axis. They answer: when a caller connects to porthole, what is
that caller allowed to do with this user's windows?

This spec pins down the policy model before `PortholeHelper.app` is built, so
the helper's menu-bar and notification UX can be designed around stable daemon
concepts instead of inventing a one-off prompt flow.

The target user experience is:

1. An agent asks to drive, observe, or manage a surface.
2. Porthole checks a local policy store.
3. If no matching grant exists, the daemon creates a pending permission request.
4. The helper prompts the user with the agent, target, actions, reason, and
   duration.
5. The user allows or denies.
6. The daemon persists the decision and unblocks or rejects the original caller.

## 2. Threat Model

Agent permissions are a cooperative local policy layer, not a hardened sandbox
against arbitrary malicious code running as the same macOS user.

The daemon still listens on a per-user Unix Domain Socket. A same-user process
that can connect to the socket is inside the operating-system trust boundary
that porthole currently uses. This design prevents accidental or over-broad
agent behavior, gives the user a clear approval surface, and gives well-behaved
agent clients a typed contract. It does not claim to stop hostile local code
that can read another process's environment, inspect files owned by the user, or
connect directly to the UDS.

That limit is deliberate. Hard isolation would require a separate OS sandbox,
per-process attestation, launchd service hardening, or a brokered XPC model.
Those can layer on later, but they are not required for the helper UX.

## 3. Terms

- **Agent identity**: a porthole-local principal such as
  `agent_a83d...`. It is the enforcement key for policy decisions.
- **Agent token**: an opaque bearer secret used by a caller to authenticate as
  an agent identity. Human-readable names, bundle paths, sessions, and tags are
  display metadata only.
- **Agent permission request**: a pending user decision for a specific identity,
  target selector, action set, and duration.
- **Grant**: a persisted allow decision.
- **Denial**: a persisted or one-shot deny decision.
- **Action class**: a named group of porthole verbs with similar user impact.
- **Target selector**: the surfaces or applications a grant applies to.

## 4. Identity

Use opaque bearer tokens for enforcement. Do not use bundle path, CLI path,
process name, `session`, or a self-reported tag as the security identity.

An agent identity record contains:

```json
{
  "agent_id": "agent_a83df4c91f0a",
  "display_name": "Flotilla Yeoman",
  "created_at": "2026-05-17T18:00:00Z",
  "created_by": "helper",
  "metadata": {
    "bundle_id": null,
    "executable_path": "/Users/robert/.local/bin/yeoman",
    "vendor": "flotilla"
  },
  "revoked_at": null
}
```

Tokens are random, non-derivable secrets. Store only a hash in porthole's local
policy store. The token may include a visible prefix for lookup, for example
`pta_agent_a83df4c91f0a.<secret>`, but the secret remains opaque to callers.

Caller requests authenticate with:

```http
Authorization: Bearer pta_agent_a83df4c91f0a.<secret>
```

Unauthenticated callers may read health-level diagnostics such as `/info`, but
they cannot observe, drive, launch, attach, record, place, replace, or close
surfaces once agent-permission enforcement is enabled.

## 5. Identity Provisioning

Identity creation is user-mediated. There are two supported provisioning paths:

- Helper UI: the user creates or approves an agent identity from the menu-bar
  app, then copies or reveals the token.
- CLI: `porthole agents create --name <name>` creates an identity and prints
  the token for the current user.

The CLI path is acceptable because porthole's current trust boundary is already
the local user account. It is not a remote enrollment endpoint. A future
hardened broker can restrict identity creation to the helper's private control
channel without changing the identity or grant model.

Agents may include optional display metadata on requests, but metadata does not
authenticate them. If a caller sends metadata without a valid token, the daemon
uses it only to make a pending setup prompt more understandable.

## 6. Scope Model

A policy decision is the tuple:

```text
agent identity + target selector + action classes + duration + constraints
```

### Target Selectors

Supported selectors:

- `surface`: one tracked `surface_id`
- `app`: surfaces whose `bundle_id`, executable path, or app name match
- `launched_by_agent`: surfaces launched by this agent identity through porthole
- `frontmost_once`: the current frontmost surface at approval time
- `all_surfaces`: explicit broad grant, shown with stronger UI language

`surface` is the default recommendation for attach/search-driven workflows.
`launched_by_agent` is the default recommendation for workflows where the agent
opens its own tool window or artifact.

### Action Classes

The first action classes should be coarse enough for users to understand:

- `observe`: screenshot, content-rect, snapshot-geometry, wait, recording read
  access, display metadata, and capture-session creation
- `drive`: focus, key, text, click, scroll, pointer movement
- `manage`: place, replace, close, attention, launch, attach, track
- `record`: long-running video recording and capture-transfer consumption

`record` is separate even though it overlaps with `observe`, because long-lived
screen capture is more sensitive than one screenshot or a stability wait.

The daemon maps each route to one or more action classes. For example,
`POST /surfaces/{id}/text` requires `drive`, `POST /launches` requires
`manage` on the `launched_by_agent` target, and
`POST /capture-sessions/surfaces/{id}` requires `observe` and `record`.

### Duration

Supported durations:

- `once`: one route call or one capture-session lifecycle
- `until_surface_gone`
- `session`: tied to the caller's porthole `session` tag
- `time_bounded`: explicit `expires_at`
- `persistent`: survives daemon restarts until revoked

Default prompts should recommend the narrowest duration that satisfies the
request. Persistent grants require explicit user selection; they are never the
default button for a first request.

### Constraints

Initial constraints:

- `requires_frontmost`: the target must be frontmost when the action executes
- `max_duration_ms`: upper bound for recording or capture sessions
- `allowed_input`: optional narrowed set for `drive` such as `["text", "key"]`

These are policy-store fields, not separate action classes, so the model can
grow without exploding the UX vocabulary.

## 7. Approval Flow

All protected actions use default-deny.

Request evaluation:

1. Authenticate caller token.
2. Resolve the route to target selector candidates and action classes.
3. Check persisted grants and denials.
4. If a matching allow exists, execute the route.
5. If a matching deny exists, return `agent_permission_denied`.
6. If no decision exists, create a pending permission request and return
   `agent_permission_needed` with the request id.

The daemon does not block an HTTP request indefinitely while waiting for the
user. Callers receive a typed error and can retry, poll the request, or listen
on `/events`.

The helper subscribes to `/events`, sees `agent_permission_requested`, presents
a notification or menu-bar sheet, then calls approve/deny endpoints. The helper
is responsible for phrasing the prompt in user terms:

```text
Flotilla Yeoman wants to type and click in "README.md - Code".
Allow for this window until it closes?
```

The approval UI must show:

- agent display name and id
- target app and surface title
- requested action classes
- duration
- reason string, if supplied
- whether the request includes recording
- buttons for Allow, Deny, and More Options

## 8. Policy Store

Use a local daemon-owned policy store under Application Support:

```text
~/Library/Application Support/Porthole/agent-policy.sqlite
```

SQLite is preferable to ad hoc JSON once approvals are mutable and queried by
helper UI. Store token hashes, identities, grants, denials, pending requests,
and an audit log.

Minimum tables:

- `agent_identities`
- `agent_tokens`
- `agent_grants`
- `agent_denials`
- `agent_permission_requests`
- `agent_permission_audit`

The audit log records who asked, what was requested, what decision was made,
when it expires, and whether the route executed after approval. It is for local
debuggability; it is not telemetry.

## 9. Wire Shape

### Caller Auth

All protected endpoints accept the same header:

```http
Authorization: Bearer <agent-token>
```

The existing JSON route bodies should not grow per-route auth fields. Identity
is transport metadata, not domain payload.

Approval, revocation, and identity-management endpoints require operator
authority, not an ordinary agent token. In the helper phase that authority
should come from a helper/private control credential minted by the daemon and
not exposed to agent clients. During CLI-only development, `porthole agents ...`
commands may act as the operator path because the current trust boundary is the
local user account, but the wire model must still distinguish operator calls
from agent calls.

### Identity Endpoints

```http
POST /agent-identities
GET  /agent-identities
GET  /agent-identities/{agent_id}
POST /agent-identities/{agent_id}/revoke
POST /agent-identities/{agent_id}/tokens
POST /agent-identities/{agent_id}/tokens/{token_id}/revoke
```

`POST /agent-identities` and token minting are user/operator operations. In the
first implementation they may be CLI-driven. When the helper exists, helper UI
should be the preferred path.

### Permission Request Endpoints

```http
GET  /agent-permissions/requests
GET  /agent-permissions/requests/{request_id}
POST /agent-permissions/requests/{request_id}/approve
POST /agent-permissions/requests/{request_id}/deny
GET  /agent-permissions/grants
POST /agent-permissions/grants/{grant_id}/revoke
```

Approve request body:

```json
{
  "duration": { "type": "until_surface_gone" },
  "target": { "type": "surface", "surface_id": "surf_123" },
  "actions": ["drive"],
  "constraints": { "requires_frontmost": false }
}
```

The approve body repeats `target` and `actions` for operator confirmation and
wire transparency. The daemon must reject mismatches and create the grant from
the source pending request's target/action scope, not from widened approve-body
values.

Deny request body:

```json
{
  "remember": false,
  "reason": "not_now"
}
```

### Error Codes

Add these wire error codes:

- `agent_identity_required` (401): missing or invalid token for a protected
  endpoint
- `agent_identity_revoked` (401): token belongs to a revoked identity
- `agent_operator_required` (403): valid caller, but the endpoint requires
  helper/operator authority
- `agent_permission_needed` (403): no matching grant; details include
  `request_id`, `target`, requested `actions`, and `surface_id` when the
  target is a concrete surface
- `agent_permission_denied` (403): matching deny or explicit user denial
- `agent_permission_request_expired` (410): caller retried against an expired
  request

Example:

```json
{
  "code": "agent_permission_needed",
  "message": "agent permission required for drive on surface surf_123",
  "details": {
    "request_id": "apr_5d1e",
    "agent_id": "agent_a83df4c91f0a",
    "target": { "type": "surface", "surface_id": "surf_123" },
    "surface_id": "surf_123",
    "actions": ["drive"],
    "recommended_duration": { "type": "until_surface_gone" }
  }
}
```

### Events

Add `/events` event types:

- `agent_permission_requested`
- `agent_permission_resolved`
- `agent_identity_created`
- `agent_identity_revoked`
- `agent_policy_changed`

Events carry ids and display metadata, not bearer tokens.

## 10. Relationship To System Permissions

System permissions remain adapter-facing OS capability checks. Agent
permissions are caller-facing product policy checks. They do not replace each
other.

Execution order for a protected route:

1. Validate request shape.
2. Authenticate agent identity.
3. Authorize agent permission.
4. Resolve surface and adapter capability.
5. Check system permission preflight.
6. Execute.

If both are missing, return the agent-permission error first. Do not trigger a
macOS TCC prompt for a caller the user has not allowed to operate the target.

Once agent permission is granted, a missing Accessibility or Screen Recording
permission still returns `system_permission_needed` with the existing
remediation details. The helper can display both states, but the wire errors
stay distinct.

## 11. Helper UX Requirements

The helper must support:

- menu-bar list of active agent identities and current grants
- notification action buttons for simple allow/deny
- expanded sheet for changing target, duration, and action scope
- revoke controls for persistent grants and agent identities
- audit history for recent decisions
- clear copy that separates "Porthole needs macOS permission" from "this agent
  wants your permission"

Notifications are useful for fast decisions, but the full menu-bar sheet is the
source of truth for non-trivial scope changes. Notification actions should only
apply the daemon's recommended narrow grant.

## 12. Defaults

Recommended defaults:

- no token: allow `/info`, deny protected routes with `agent_identity_required`
- new identity: no grants
- first request for launched surface: recommend `launched_by_agent` +
  requested action class + `until_surface_gone`
- first request for existing surface: recommend `surface` + requested action
  class + `until_surface_gone`
- recording request: recommend `surface` + `record` + explicit max duration
- persistent grant: available under More Options, never primary
- broad `all_surfaces`: available only under More Options with stronger warning

## 13. Testing Strategy

The implementation should be testable without macOS UI:

- pure policy-engine tests for selector matching, action-class matching,
  duration expiry, revocation, and denial precedence
- route tests against `InMemoryAdapter` proving protected endpoints check agent
  policy before adapter calls
- serialization tests for new error details and event bodies
- helper-less CLI tests for identity creation and grant listing
- manual helper smoke only when the Swift helper exists

Permission-dependent adapter tests keep the existing repo rule: if macOS
Accessibility or Screen Recording is missing, stop `BLOCKED` and wait for the
grant. Do not add policy bypasses to make those tests pass.

## 14. Non-Goals

- No network authentication or remote multi-user authorization.
- No cryptographic proof that a token holder is a specific process binary.
- No OS sandboxing of same-user malicious code.
- No mapping of agent permissions onto macOS TCC.
- No helper UI implementation in Phase 2.
- No compatibility mode for unauthenticated protected routes once enforcement
  ships; porthole is pre-release.

## 15. Implementation Order

1. Build a pure Rust policy model and policy-store abstraction.
2. Add identity/token management and CLI commands.
3. Add route middleware or per-route guards for protected endpoints.
4. Add pending permission request creation and `/events` publication.
5. Build helper UI on top of the stable endpoints.
6. Add notification actions for approve/deny.
7. Add revocation and audit UI.

The helper should not invent private policy state. It should display and mutate
daemon-owned identities, requests, and grants through the wire shape above.

## 16. Success Criterion

A new agent can be given a token, request permission to drive a specific
window, receive a default-deny error with a request id, wait while the helper
prompts the user, retry after approval, and then operate only within the target,
action, and duration the user granted.
