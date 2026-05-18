# Capture Transfer FD Auth Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bearer-token handshaking to raw capture-transfer fd-socket consumption for protected surface capture sessions.

**Architecture:** The existing capture-transfer fd socket stays in place. Surface capture sessions store the creating agent id, fd connections authenticate once with an `authorize` message, and frame acquisition checks the connection identity against the requested session owner. Synthetic sessions remain ownerless and unauthenticated.

**Tech Stack:** Rust, serde-tagged JSON protocol, Unix-domain sockets with fd passing, porthole agent policy store, existing cargo workspace gates.

---

## Files

- Modify: `crates/capture-transfer/src/transfer_channel.rs`
- Modify: `crates/capture-transfer/src/daemon.rs`
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `crates/portholed/src/routes/capture_sessions.rs`
- Modify: `crates/portholed/src/server.rs`
- Modify: `crates/porthole/src/client.rs`
- Modify: `crates/porthole/src/commands/record.rs`
- Modify: `docs/roadmap.md`
- Test: existing unit and integration tests in the same files

## Task 1: Wire Message

- [x] Add `CaptureTransferRequest::Authorize { session_id, bearer_token }`.
- [x] Update `request_messages_roundtrip_with_snake_case_ops` to expect `op: "authorize"`.
- [x] Run `cargo test -p capture-transfer transfer_channel::tests::request_messages_roundtrip_with_snake_case_ops --locked` and verify it passes.

## Task 2: Session Ownership and FD Authorization

- [x] Add `owner_agent_id: Option<AgentId>` to `CaptureSession`.
- [x] Add a `CaptureRegistry` clone of `AgentPolicyStore`, or an optional authenticator object, so fd threads can authenticate tokens.
- [x] Pass the authorized route execution's `agent_id` from `POST /capture-sessions/surfaces/{id}` into `create_surface_session`.
- [x] Keep synthetic sessions ownerless.
- [x] Add tests proving protected sessions reject unauthenticated and wrong-token fd frame acquisition.
- [x] Add tests proving matching-token authorization allows frame acquisition.
- [x] Run targeted `portholed` fd-channel tests.

## Task 3: Client Token Propagation

- [x] Expose the `DaemonClient` bearer token as an optional clone.
- [x] Add `bearer_token: Option<String>` to `RecordSession` and `capture_transfer::daemon::SessionInfo`.
- [x] Have `DaemonConsumer::connect` send `authorize` when `SessionInfo::bearer_token` is present.
- [x] Pass the CLI agent token from surface recording session creation into the fd consumer.
- [x] Update record and capture-transfer daemon tests for the new field.

## Task 4: Docs and Gates

- [x] Update `docs/roadmap.md` to mark the raw fd bearer-auth item complete.
- [x] Run:
  - `cargo build --workspace --locked`
  - `cargo test --workspace --locked`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo +nightly-2026-03-12 fmt --check`
  - `git diff --check`
- [x] Commit, push, open PR, and shepherd it through checks/review.
