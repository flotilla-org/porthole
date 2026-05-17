# Agent Permissions Core Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add daemon-owned agent identity, token, grant, denial, and pending-request enforcement so protected routes default-deny without an approved local policy decision.

**Architecture:** Keep policy truth in Rust, owned by `portholed`. Add pure policy types and matching logic in `porthole-core`, wire structs in `porthole-protocol`, a SQLite-backed policy store in `portholed`, SSE events for pending/resolved permission state, and CLI/operator commands in `porthole` for the first non-helper administration path. Start with a complete vertical slice for drive actions on tracked surfaces, then make the route guard/mapping extensible for observe/manage/record in follow-up slices.

**Tech Stack:** Rust 2024, axum extractors, serde wire structs, rusqlite for local policy persistence, sha2 for token hashing, uuid for ids, existing HTTP-over-UDS client/daemon tests.

---

## Scope And Concerns

This plan intentionally does **not** build the macOS notification UI. The notification surface depends on real pending agent-permission requests, approval endpoints, and route enforcement. Building notifications first would create UI around a policy layer that does not exist.

This plan also does **not** protect every route in one PR. It builds the full policy model, persistence, identity/token management, approval endpoints, `/events` publication for permission state, and a complete protected vertical slice for `drive` on surface input routes. Follow-up PRs should map remaining route classes (`observe`, `manage`, `record`) once the guard is proven.

No OS permission workarounds are allowed. If an implementation task trips a real macOS Accessibility or Screen Recording call and the grant is missing, stop `BLOCKED` per `AGENTS.md`.

## File Structure

- Modify `Cargo.toml`
  - Add workspace deps `rusqlite`, `sha2`, `hex`, and `subtle`.
- Modify `crates/porthole-core/src/error.rs`
  - Add agent-permission wire error codes.
- Create `crates/porthole-core/src/agent_policy.rs`
  - Pure ids, action classes, target selectors, durations, decisions, request structs, and matching engine.
- Modify `crates/porthole-core/src/lib.rs`
  - Export `agent_policy` types.
- Create `crates/porthole-protocol/src/agent_permissions.rs`
  - Request/response bodies for identities, grants, pending requests, approve/deny, and error details.
- Modify `crates/porthole-protocol/src/lib.rs`
  - Export agent-permission wire module.
- Create `crates/portholed/src/agent_store.rs`
  - SQLite schema, token hashing, identity/token/grant/denial/request/audit persistence.
- Create `crates/portholed/src/events.rs`
  - In-process event bus and SSE payloads for agent permission and identity events.
- Modify `crates/portholed/src/state.rs`
  - Add `agent_store` and `events` to `AppState`; provide test constructors using tempfile-backed or in-memory SQLite.
- Create `crates/portholed/src/routes/agent_permissions.rs`
  - Identity endpoints, pending-request endpoints, approve/deny/revoke endpoints.
- Create `crates/portholed/src/routes/events.rs`
  - `GET /events` SSE endpoint.
- Create `crates/portholed/src/routes/agent_guard.rs`
  - Header extraction, `authorize_or_create_request`, audit logging, and event publication helper used by protected routes.
- Modify `crates/portholed/src/server.rs`
  - Mount agent routes, mount `/events`, and apply drive guard to input/pointer routes.
- Modify `crates/portholed/src/routes/input.rs` and `crates/portholed/src/routes/pointer.rs`
  - Authorize `drive` before adapter/pipeline calls.
- Modify `crates/portholed/src/routes/errors.rs`
  - Map new error codes to 401/403/410.
- Modify `crates/porthole/src/client.rs`
  - Add optional bearer-token support and raw auth-post helpers.
- Modify `crates/porthole/src/main.rs` and `crates/porthole/src/commands/mod.rs`
  - Add `porthole agents ...` CLI commands.
- Create `crates/porthole/src/commands/agents.rs`
  - Create/list identities, list requests, approve/deny request, list grants.
- Modify `docs/roadmap.md`
  - Add/check a subitem for agent-permission enforcement foundation only after gates pass.
- Modify `docs/superpowers/specs/2026-05-17-porthole-agent-permissions-design.md`
  - Add the operator grant-list endpoint and the approve-body target/action validation rule if the implementation PR has not already landed those spec clarifications.

---

## Chunk 1: Pure Policy Model And Wire Errors

### Task 1: Add Agent Error Codes

**Files:**
- Modify: `crates/porthole-core/src/error.rs`
- Modify: `crates/portholed/src/routes/errors.rs`

- [ ] **Step 1: Add failing error-code tests**

Add to `crates/porthole-core/src/error.rs` tests:

```rust
#[test]
fn agent_permission_error_codes_display_as_snake_case() {
    assert_eq!(ErrorCode::AgentIdentityRequired.to_string(), "agent_identity_required");
    assert_eq!(ErrorCode::AgentIdentityRevoked.to_string(), "agent_identity_revoked");
    assert_eq!(ErrorCode::AgentOperatorRequired.to_string(), "agent_operator_required");
    assert_eq!(ErrorCode::AgentPermissionNeeded.to_string(), "agent_permission_needed");
    assert_eq!(ErrorCode::AgentPermissionDenied.to_string(), "agent_permission_denied");
    assert_eq!(ErrorCode::AgentPermissionRequestExpired.to_string(), "agent_permission_request_expired");
}
```

Add to `crates/portholed/src/routes/errors.rs` tests:

```rust
#[test]
fn agent_permission_errors_map_to_auth_statuses() {
    assert_eq!(ApiError(WireError { code: ErrorCode::AgentIdentityRequired, message: "missing".into(), details: None }).into_response().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(ApiError(WireError { code: ErrorCode::AgentIdentityRevoked, message: "revoked".into(), details: None }).into_response().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(ApiError(WireError { code: ErrorCode::AgentOperatorRequired, message: "operator".into(), details: None }).into_response().status(), StatusCode::FORBIDDEN);
    assert_eq!(ApiError(WireError { code: ErrorCode::AgentPermissionNeeded, message: "needed".into(), details: None }).into_response().status(), StatusCode::FORBIDDEN);
    assert_eq!(ApiError(WireError { code: ErrorCode::AgentPermissionDenied, message: "denied".into(), details: None }).into_response().status(), StatusCode::FORBIDDEN);
    assert_eq!(ApiError(WireError { code: ErrorCode::AgentPermissionRequestExpired, message: "expired".into(), details: None }).into_response().status(), StatusCode::GONE);
}
```

- [ ] **Step 2: Verify red**

Run:

```sh
cargo test -p porthole-core agent_permission_error_codes_display_as_snake_case --locked
cargo test -p portholed agent_permission_errors_map_to_auth_statuses --locked
```

Expected: FAIL because the variants do not exist.

- [ ] **Step 3: Implement error codes and status mapping**

Add variants to `ErrorCode`, update `Display`, and update `ApiError::into_response` with the statuses from the spec.

- [ ] **Step 4: Verify green**

Run the same two tests. Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add crates/porthole-core/src/error.rs crates/portholed/src/routes/errors.rs
git commit -m "feat: add agent permission error codes"
```

### Task 2: Add Pure Policy Model

**Files:**
- Create: `crates/porthole-core/src/agent_policy.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Add failing policy tests**

Create `crates/porthole-core/src/agent_policy.rs` with tests first. Cover:

```rust
#[test]
fn allow_grant_matches_agent_surface_and_action() { /* grant drive on surf_1 authorizes drive on surf_1 */ }

#[test]
fn denial_takes_precedence_over_allow() { /* matching denial returns denied even with matching grant */ }

#[test]
fn expired_grant_does_not_authorize() { /* expires_at before now is ignored */ }

#[test]
fn launched_by_agent_selector_matches_agent_owned_surface() { /* target selector based on launch owner */ }

#[test]
fn app_selector_matches_bundle_executable_or_app_name() { /* app selector supports all spec-defined app keys */ }

#[test]
fn frontmost_once_selector_matches_only_the_approved_frontmost_surface() { /* approval captures concrete surface */ }

#[test]
fn constraints_are_carried_on_grants() { /* constraints are modelled even before every route uses them */ }

#[test]
fn all_surfaces_selector_authorizes_any_surface_for_agent() { /* broad grant is explicit and test-covered */ }

#[test]
fn once_grant_marked_consumed_is_not_authorized_again() { /* once duration does not behave like persistent */ }
```

Use concrete structs named in the implementation step below so the first compile fails on missing types.

- [ ] **Step 2: Verify red**

Run:

```sh
cargo test -p porthole-core agent_policy --locked
```

Expected: FAIL on missing policy types.

- [ ] **Step 3: Implement pure types**

Implement:

```rust
pub struct AgentId(String);
pub struct GrantId(String);
pub struct DenialId(String);
pub struct PermissionRequestId(String);

pub enum ActionClass { Observe, Drive, Manage, Record }
pub enum AppSelector {
    BundleId(String),
    ExecutablePath(String),
    AppName(String),
}
pub enum TargetSelector {
    Surface { surface_id: SurfaceId },
    App { app: AppSelector },
    LaunchedByAgent,
    FrontmostOnce { surface_id: SurfaceId },
    AllSurfaces,
}
pub enum DurationSpec { Once, UntilSurfaceGone, Session { session: String }, TimeBounded { expires_at_unix_ms: u64 }, Persistent }
pub enum Constraint {
    RequiresFrontmost,
    MaxDurationMs(u64),
    AllowedInput(Vec<String>),
}
pub enum AuthorizationDecision { Allowed { grant_id: GrantId, consumes_grant: bool }, Denied { denial_id: DenialId }, NeedsPermission }

pub struct AgentContext { pub agent_id: AgentId }
pub struct TargetContext {
    pub surface_id: Option<SurfaceId>,
    pub app_bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub app_name: Option<String>,
    pub launched_by_agent: Option<AgentId>,
    pub frontmost_surface_id: Option<SurfaceId>,
    pub surface_alive: bool,
}
pub struct Grant { pub constraints: Vec<Constraint>, ... }
pub struct Denial { ... }
pub struct PolicySnapshot { pub grants: Vec<Grant>, pub denials: Vec<Denial>, pub consumed_once_grants: Vec<GrantId> }
impl PolicySnapshot { pub fn authorize(&self, agent: &AgentContext, target: &TargetContext, actions: &[ActionClass], now_unix_ms: u64) -> AuthorizationDecision }
```

Keep matching pure and deterministic. Do not introduce persistence here. `once` consumption is reserved by the store before route execution; the pure model should still expose whether a grant is consumable and ignore grant ids listed in `consumed_once_grants`.

`DurationSpec::Session` should stay in the type because it is in the approved wire model, but implementation should add a doc comment on the variant stating that session matching is reserved until the daemon has an explicit porthole session tag. Until then, matching should treat `Session` grants as not authorized rather than silently broadening them.

- [ ] **Step 4: Export and verify**

Update `lib.rs` with `pub mod agent_policy;` and run:

```sh
cargo test -p porthole-core agent_policy --locked
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add crates/porthole-core/src/agent_policy.rs crates/porthole-core/src/lib.rs
git commit -m "feat: model agent permission policy"
```

---

## Chunk 2: Wire Types And SQLite Store

### Task 3: Add Wire Bodies

**Files:**
- Create: `crates/porthole-protocol/src/agent_permissions.rs`
- Modify: `crates/porthole-protocol/src/lib.rs`

- [ ] **Step 1: Add serialization tests**

Cover:

- `CreateAgentIdentityRequest { display_name, metadata: Option<AgentIdentityMetadata> }`
- `CreateAgentIdentityResponse { agent_id, token }`
- `MintAgentTokenResponse { token_id, token }`
- `AgentIdentityMetadata { bundle_id, executable_path, vendor }`, with each field optional
- `AgentPermissionNeededDetails { request_id, agent_id, surface_id, actions, recommended_duration }`, including round-trip coverage for the nested duration shape
- `ApproveAgentPermissionRequest { duration, target, actions, constraints }`
- `DenyAgentPermissionRequest { remember, reason }`

- [ ] **Step 2: Verify red**

Run:

```sh
cargo test -p porthole-protocol agent_permissions --locked
```

Expected: FAIL on missing module/types.

- [ ] **Step 3: Implement serde structs**

Use `#[serde(rename_all = "snake_case")]` for enum tags and field names matching the approved spec. Include optional identity metadata (`bundle_id`, `executable_path`, `vendor`) from the start even though metadata does not authenticate the caller. Use core newtypes where possible; otherwise serialize ids as strings and convert in route code.

- [ ] **Step 4: Verify green and commit**

```sh
cargo test -p porthole-protocol agent_permissions --locked
git add crates/porthole-protocol/src/agent_permissions.rs crates/porthole-protocol/src/lib.rs
git commit -m "feat: add agent permission wire types"
```

### Task 4: Add SQLite Policy Store

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/portholed/Cargo.toml`
- Create: `crates/portholed/src/agent_store.rs`
- Modify: `crates/portholed/src/lib.rs`

- [ ] **Step 1: Add failing store tests**

Create tests in `agent_store.rs` for:

- schema initializes in a temp DB
- creating an identity returns an agent id and one plaintext token only once
- authenticating with the plaintext token returns the agent id
- token hashes, not plaintext tokens, are stored
- creating a pending request is idempotent for same agent/target/action while pending
- pending request idempotency canonicalizes action ordering so `[Drive, Observe]` and `[Observe, Drive]` reuse the same pending request
- approving a request creates a matching grant and marks request resolved
- denial takes precedence in the store snapshot
- audit rows are written for requested, approved, denied, and executed decisions
- once grant is atomically consumed/reserved before execution and a second concurrent route cannot also execute it
- token lookup parses the visible `agent_id` prefix and does not full-scan token hashes

- [ ] **Step 2: Add dependencies and verify red**

Add workspace deps:

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
sha2 = "0.10"
hex = "0.4"
subtle = "2"
```

Add `rusqlite`, `sha2`, `hex`, and `subtle` to `portholed` dependencies. Then run:

Keep `rusqlite`'s `bundled` feature for this slice so CI and developer machines do not depend on system SQLite headers or OS-specific SQLite versions. The binary-size cost is acceptable for the daemon at this stage.

```sh
cargo test -p portholed agent_store --locked
```

Expected: FAIL until `Cargo.lock` is updated or types are missing. If `--locked` fails only because new dependencies need lockfile entries, run the same command without `--locked` once to update `Cargo.lock`, then rerun with `--locked`.

The updated `Cargo.lock` is part of this task. Commit it with the dependency change; otherwise every later `--locked` gate and CI run will fail.

- [ ] **Step 3: Implement store**

Implement `AgentPolicyStore` as a cloneable wrapper around `Arc<Mutex<rusqlite::Connection>>`. Because `rusqlite` is synchronous, expose async store methods that run the mutex-bound database work inside `tokio::task::spawn_blocking` or a dedicated blocking thread pool. Do not run SQLite calls on axum's async worker threads, and do not hold the mutex across `.await`.

The single mutex is acceptable for this first local policy store because guard calls are short, local, and serialized through one daemon process. If measurements show policy-store contention, replace the connection wrapper with an SQLite pool such as `r2d2`, `deadpool-sqlite`, or an equivalent local pool without changing policy semantics.

Production `portholed` opens the policy database at `~/Library/Application Support/Porthole/agent-policy.sqlite` on macOS, creating the parent directory with owner-only permissions where possible before opening the connection. Test constructors use tempfile-backed or in-memory SQLite and must not touch the production path.

Schema tables for this slice:

- `agent_identities(agent_id primary key, display_name, metadata_json, created_at_unix_ms, revoked_at_unix_ms null)`
- `agent_tokens(token_id primary key, agent_id, token_hash, created_at_unix_ms, revoked_at_unix_ms null)`
- `agent_grants(grant_id primary key, agent_id, origin_request_id null, target_json, actions_json, duration_json, constraints_json, created_at_unix_ms, expires_at_unix_ms null, consumed_at_unix_ms null, revoked_at_unix_ms null)`
- `agent_denials(denial_id primary key, agent_id, target_json, actions_json, created_at_unix_ms, expires_at_unix_ms null)`
- `agent_permission_requests(request_id primary key, agent_id, target_json, actions_json, reason null, status, created_at_unix_ms, resolved_at_unix_ms null)`
- `agent_permission_audit(audit_id primary key, agent_id, request_id null, route, target_json, actions_json, decision, grant_id null, denial_id null, requested_at_unix_ms null, decided_at_unix_ms null, expires_at_unix_ms null, executed_at_unix_ms null)`

Schema indexes/constraints for this slice:

- `CREATE INDEX agent_tokens_agent_id ON agent_tokens(agent_id)`
- `CREATE UNIQUE INDEX agent_permission_requests_pending_unique ON agent_permission_requests(agent_id, target_json, actions_json) WHERE status = 'pending'`
- `CREATE UNIQUE INDEX agent_grants_origin_request_unique ON agent_grants(origin_request_id) WHERE origin_request_id IS NOT NULL`

Before serializing action sets into `actions_json` for requests, grants, denials, or audit rows, canonicalize them by sorting on a fixed `ActionClass` order (`Observe`, `Drive`, `Manage`, `Record`) and deduping duplicate action classes. Store and matching tests should include shuffled action input to prove that logically equivalent action sets serialize identically and hit the same pending-request unique index.

Token format: `pta_<agent_id>.<secret_uuid_without_hyphens>`. Store only `sha256(token)` as hex. Authentication should parse the `agent_id` prefix, query that identity's non-revoked tokens, and then compare the hash against those candidates with `subtle::ConstantTimeEq`; do not full-scan all stored token hashes.

When approving a request, copy the source `request_id` into `agent_grants.origin_request_id`. When loading `PolicySnapshot`, treat grants with `duration = once` and non-null `consumed_at_unix_ms` as consumed by adding their ids to `consumed_once_grants`.

Add a store method `try_consume_once_grant(grant_id, consumed_at_unix_ms)` that only marks grants whose duration is `once` using a conditional update equivalent to:

```sql
UPDATE agent_grants
SET consumed_at_unix_ms = ?
WHERE grant_id = ?
  AND consumed_at_unix_ms IS NULL
  AND revoked_at_unix_ms IS NULL
```

The implementation must also verify the grant's decoded `duration_json` is `DurationSpec::Once` in the same blocking transaction before updating. The method returns whether one row was updated. It must return false for already-consumed, persistent, time-bounded, or revoked grants.

- [ ] **Step 4: Verify green and commit**

```sh
cargo test -p portholed agent_store --locked
git add Cargo.toml Cargo.lock crates/portholed/Cargo.toml crates/portholed/src/agent_store.rs crates/portholed/src/lib.rs
git commit -m "feat: persist agent permission policy"
```

---

## Chunk 3: Daemon Endpoints And Route Guard

### Task 5: Mount Identity And Permission Endpoints

**Files:**
- Create: `crates/portholed/src/routes/agent_permissions.rs`
- Modify: `crates/portholed/src/routes/mod.rs`
- Modify: `crates/portholed/src/server.rs`
- Modify: `crates/portholed/src/state.rs`

- [ ] **Step 1: Add route tests**

In `server.rs` tests or `routes/agent_permissions.rs` tests, cover:

- `POST /agent-identities` creates identity and returns token
- `GET /agent-identities` lists identities without tokens
- `GET /agent-identities/{agent_id}` returns identity detail without tokens
- `POST /agent-identities/{agent_id}/revoke` revokes identity and all active tokens
- `POST /agent-identities/{agent_id}/tokens` mints an additional token once and returns both `token_id` and plaintext `token`
- `POST /agent-identities/{agent_id}/tokens/{token_id}/revoke` revokes one token
- `GET /agent-permissions/requests` returns pending requests
- `GET /agent-permissions/requests/{id}` returns one request and status
- `POST /agent-permissions/requests/{id}/approve` resolves a request and creates a grant
- `POST /agent-permissions/requests/{id}/approve` with a body whose `target` or `actions` differ from the pending request returns 400 `invalid_argument` and does not create a grant
- `POST /agent-permissions/requests/{id}/deny` resolves a request and creates a denial or one-shot deny
- `GET /agent-permissions/grants` lists active and revoked grants for operator inspection
- `POST /agent-permissions/grants/{grant_id}/revoke` revokes a grant
- approving, denying, creating, and revoking publishes the matching `/events` payload

Use the current local-user trust boundary for operator calls in this slice: no operator credential required yet, but keep route helpers named so a helper credential can be inserted later.

Test event publication through the in-process event bus, not by waiting on an infinite SSE response in oneshot router tests. Route tests should subscribe to or inspect the test event bus, perform the operator action, and assert the queued payload. Add a separate focused test for `GET /events` stream framing that consumes one already-published event and then drops the stream.

- [ ] **Step 2: Verify red**

Run:

```sh
cargo test -p portholed agent_permission_routes --locked
```

Expected: FAIL on missing routes.

- [ ] **Step 3: Implement routes**

Mount:

```rust
.route("/agent-identities", post(agent_permissions_route::post_identity).get(agent_permissions_route::get_identities))
.route("/agent-identities/{agent_id}", get(agent_permissions_route::get_identity))
.route("/agent-identities/{agent_id}/revoke", post(agent_permissions_route::post_revoke_identity))
.route("/agent-identities/{agent_id}/tokens", post(agent_permissions_route::post_identity_token))
.route("/agent-identities/{agent_id}/tokens/{token_id}/revoke", post(agent_permissions_route::post_revoke_identity_token))
.route("/agent-permissions/requests", get(agent_permissions_route::get_requests))
.route("/agent-permissions/requests/{request_id}", get(agent_permissions_route::get_request))
.route("/agent-permissions/requests/{request_id}/approve", post(agent_permissions_route::post_approve_request))
.route("/agent-permissions/requests/{request_id}/deny", post(agent_permissions_route::post_deny_request))
.route("/agent-permissions/grants", get(agent_permissions_route::get_grants))
.route("/agent-permissions/grants/{grant_id}/revoke", post(agent_permissions_route::post_revoke_grant))
.route("/events", get(events_route::get_events))
```

Return wire structs from `porthole-protocol::agent_permissions`. The `/events` payloads for this slice are `agent_permission_requested`, `agent_permission_resolved`, `agent_identity_created`, `agent_identity_revoked`, and `agent_policy_changed`; events carry ids and display metadata, never bearer tokens.

When approving a request, the approve body must include `target` and `actions` to match the approved spec, but the route must validate that they exactly match the source pending request. The stored grant uses target/actions from the source request, not untrusted widened values from the approve body. Mismatches return `invalid_argument`.

Implement the event bus with `tokio::sync::broadcast` capacity 256. If an SSE subscriber receives `RecvError::Lagged`, log the lag, emit or send a `resync_required` SSE event if the connection is still open, and continue streaming new events. Helper/UI clients must recover missed permission events by polling `GET /agent-permissions/requests`, so a dropped event must not leave the UI permanently inconsistent.

Add a bounded lag/resync test by constructing a small-capacity test bus or otherwise forcing a receiver to fall behind, then assert that `GET /events` yields `resync_required` and can continue with later events.

- [ ] **Step 4: Verify green and commit**

```sh
cargo test -p portholed agent_permission_routes --locked
git add crates/portholed/src/events.rs crates/portholed/src/routes/agent_permissions.rs crates/portholed/src/routes/events.rs crates/portholed/src/routes/mod.rs crates/portholed/src/server.rs crates/portholed/src/state.rs
git commit -m "feat: add agent permission daemon endpoints"
```

### Task 6: Add Authorization Guard For Drive Routes

**Files:**
- Create: `crates/portholed/src/routes/agent_guard.rs`
- Modify: `crates/portholed/src/routes/input.rs`
- Modify: `crates/portholed/src/routes/pointer.rs`
- Modify: `crates/portholed/src/routes/mod.rs`

- [ ] **Step 1: Add route-guard tests**

Use `InMemoryAdapter` router tests:

- no `Authorization` header on `POST /surfaces/{id}/text` returns 401 `agent_identity_required`
- invalid bearer token returns 401 `agent_identity_required`
- valid identity without grant returns 403 `agent_permission_needed` and creates a pending request id
- first missing-grant request publishes `agent_permission_requested` on `/events`
- approving that request allows retry of the same `text` call
- denied request returns 403 `agent_permission_denied`
- successful retry after approval writes an audit row with `executed_at_unix_ms`
- `/info` remains unauthenticated

As in Task 5, assert event publication against the event bus directly in guard tests. Do not write a test that waits indefinitely on `/events`; only the dedicated SSE framing test should touch the streaming route, and it must publish one event before connecting or use a bounded timeout.

- [ ] **Step 2: Verify red**

Run:

```sh
cargo test -p portholed agent_guard --locked
```

Expected: FAIL because routes are still unprotected.

- [ ] **Step 3: Implement guard**

Add helper:

```rust
pub struct AuthorizedRouteExecution {
    pub agent_id: AgentId,
    pub grant_id: GrantId,
    pub origin_request_id: Option<PermissionRequestId>,
    pub consumes_grant: bool,
    pub route: &'static str,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
}

pub async fn authorize_surface_actions(
    state: &AppState,
    headers: &HeaderMap,
    surface_id: &str,
    actions: &[ActionClass],
    reason: Option<&str>,
) -> Result<AuthorizedRouteExecution, ApiError>

pub async fn complete_route_execution(
    state: &AppState,
    execution: AuthorizedRouteExecution,
) -> Result<(), ApiError>
```

Behavior:

1. Parse `Authorization: Bearer ...`.
2. Authenticate token through `AgentPolicyStore`.
3. Resolve the tracked surface before adapter/system-permission calls.
4. Authorize against the policy snapshot.
5. If no grant/deny matches, create or reuse a pending request, publish `agent_permission_requested` only when a new request is inserted, insert a request audit row, and return `agent_permission_needed` with details. The `recommended_duration` in the error details should be selected in this guard/request-creation layer: recommend `TargetSelector::LaunchedByAgent` with `DurationSpec::UntilSurfaceGone` when the tracked surface was launched by the agent, otherwise default to `TargetSelector::Surface` with `DurationSpec::UntilSurfaceGone` for this drive-route slice.
6. If a grant matches and `consumes_grant` is true, atomically reserve it before route execution with `AgentPolicyStore::try_consume_once_grant`. If the conditional update affects zero rows, another concurrent route consumed the grant; reload authorization state and return `agent_permission_needed` or `agent_permission_denied` as appropriate rather than executing the route.
7. If a grant matches, return `AuthorizedRouteExecution` to the route handler instead of writing execution state immediately.
8. Each protected route must call `complete_route_execution` only after the adapter/pipeline call succeeds. That helper inserts a separate execution audit row with `executed_at_unix_ms`, `grant_id`, and the grant's `origin_request_id` when present.
9. If `complete_route_execution` cannot write audit state after the route action already happened, log the error and return `Ok(())`; do not report a post-execution store failure as a failed user action.

Apply to input and pointer movement routes first. Keep this function reusable for observe/manage/record mappings in later PRs.

- [ ] **Step 4: Verify green and commit**

```sh
cargo test -p portholed agent_guard --locked
git add crates/portholed/src/routes/agent_guard.rs crates/portholed/src/routes/input.rs crates/portholed/src/routes/pointer.rs crates/portholed/src/routes/mod.rs
git commit -m "feat: guard drive routes with agent policy"
```

---

## Chunk 4: CLI Operator Commands

### Task 7: Add Bearer Support To CLI Client

**Files:**
- Modify: `crates/porthole/src/client.rs`

- [ ] **Step 1: Add client tests**

Add tests for request construction proving a bearer token is added when configured and omitted by default. If direct hyper request inspection is awkward, split request-building into a small private helper and test that helper.

- [ ] **Step 2: Implement**

Add `DaemonClient::with_bearer_token(socket, token)` as a builder-style constructor/modifier. Ensure all `get_json`, `post_json`, and `delete_empty` paths attach the header.

- [ ] **Step 3: Verify and commit**

```sh
cargo test -p porthole client --locked
git add crates/porthole/src/client.rs
git commit -m "feat: support agent bearer tokens in CLI client"
```

### Task 8: Add `porthole agents` Commands

**Files:**
- Modify: `crates/porthole/src/main.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Create: `crates/porthole/src/commands/agents.rs`
- Add tests under `crates/porthole/tests/agents_cli.rs`

- [ ] **Step 1: Add CLI rendering tests**

Cover:

- `porthole agents create --name test --json` renders token once
- `porthole agents show <agent_id> --json` renders identity metadata without token secrets
- `porthole agents token create <agent_id> --json` renders a second token once
- `porthole agents revoke <agent_id>` and `porthole agents token revoke <agent_id> <token_id>` call the revocation endpoints
- `porthole agents grants --json` renders active grants
- `porthole agents grant revoke <grant_id>` calls the grant revocation endpoint
- `porthole agents requests --json` renders pending request ids
- `porthole agents request <request_id> --json` renders one request status
- `porthole agents approve <request_id> --duration until-surface-gone`
- `porthole agents deny <request_id> --reason not_now`

Use test-local fake daemon responses following existing CLI test patterns.

- [ ] **Step 2: Verify red**

Run:

```sh
cargo test -p porthole agents_cli --locked
```

Expected: FAIL on missing command.

- [ ] **Step 3: Implement commands**

Add subcommands:

```text
porthole agents create --name <display-name> [--json]
porthole agents list [--json]
porthole agents show <agent_id> [--json]
porthole agents revoke <agent_id> [--json]
porthole agents token create <agent_id> [--json]
porthole agents token revoke <agent_id> <token_id> [--json]
porthole agents grants [--json]
porthole agents grant revoke <grant_id> [--json]
porthole agents requests [--json]
porthole agents request <request_id> [--json]
porthole agents approve <request_id> --duration until-surface-gone [--json]
porthole agents deny <request_id> [--remember] [--reason <reason>] [--json]
```

Do not persist agent tokens in CLI config in this slice. The token prints once on create; users pass it to agent clients via their own secret mechanism.

The approve command must first `GET /agent-permissions/requests/{request_id}` to read the pending request's `target` and `actions`, then include those values in the approve body alongside the operator-supplied `duration` and optional constraints. CLI tests must assert that the fake daemon receives target/actions derived from the pre-fetched request, not invented by CLI flags.

The CLI approval command does not expose constraint editing in this slice; it sends an empty/default `constraints` value unless future flags are added.

- [ ] **Step 4: Verify green and commit**

```sh
cargo test -p porthole agents_cli --locked
git add crates/porthole/src/main.rs crates/porthole/src/commands/mod.rs crates/porthole/src/commands/agents.rs crates/porthole/tests/agents_cli.rs
git commit -m "feat: add agent permission CLI commands"
```

---

## Chunk 5: Docs, Roadmap, And Gates

### Task 9: Roadmap And README Notes

**Files:**
- Modify: `docs/roadmap.md`
- Modify: `README.md`

- [ ] **Step 1: Update docs**

Document the first operator flow:

```sh
porthole agents create --name "My Agent" --json
# pass the returned token as Authorization: Bearer ...
porthole agents requests --json
porthole agents approve <request_id> --duration until-surface-gone
porthole agents grants --json
porthole agents grant revoke <grant_id>
```

Document that helper/UI clients should subscribe to `/events` for `agent_permission_requested` and `agent_permission_resolved` rather than polling in the steady state. The CLI may still support polling via `porthole agents requests` and `porthole agents request <request_id>`.

Update roadmap to add/check only an enforcement-foundation item. Update the agent-permissions spec if needed for `GET /agent-permissions/grants` and approve-body target/action validation. Do not tick notification approvals until helper UI consumes the request endpoints.

- [ ] **Step 2: Run full verification**

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all PASS. Permission-gated ignored tests remain ignored.

- [ ] **Step 3: Commit docs**

```sh
git add docs/roadmap.md README.md
git commit -m "docs: document agent permission enforcement foundation"
```

### Task 10: PR Checklist

Before opening the PR, include:

- Summary of policy model, SQLite store including audit log, identity endpoints, `/events` publication, route guard, and CLI operator commands.
- Explicit note that notification UI is not included.
- Explicit note that only drive input/pointer routes are protected in this first enforcement PR, with follow-up route-class mapping for observe/manage/record.
- Validation commands from Task 9.
- Manual permission note: no live macOS TCC grant behavior was exercised.
