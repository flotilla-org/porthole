# Launch Force Place Preexisting Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `force_place: true` to launch/replace requests so callers can explicitly apply placement to preexisting surfaces that would otherwise be tracked but not moved.

**Architecture:** Extend the existing launch request/spec boolean set with `force_place`, defaulting to false. `LaunchPipeline` keeps the current safety default (`SkippedPreexisting`) unless a non-empty placement is requested and `force_place` is true; `require_fresh_surface && force_place` is rejected as invalid because one asks to fail on preexisting surfaces while the other asks to move them.

**Tech Stack:** Rust core/protocol/daemon/CLI, serde defaults, existing in-memory adapter route tests, cargo workspace CI.

---

## Context

This implements the deferred v0.1 item named in `docs/superpowers/specs/2026-04-20-porthole-design.md` §6.6 and `docs/superpowers/specs/2026-04-22-porthole-slice-c-design.md` §11:

- default: placement on preexisting launch-correlated surfaces is skipped
- `force_place: true`: caller explicitly overrides that skip in one launch call

## Files

- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/launch.rs`
- Modify: `crates/porthole-protocol/src/launches.rs`
- Modify: `crates/portholed/src/routes/launches.rs`
- Modify: `crates/portholed/src/routes/replace.rs`
- Modify: `crates/portholed/src/server.rs`
- Modify: `crates/porthole/src/commands/launch.rs`
- Modify: `crates/porthole/src/main.rs`
- Modify: `docs/roadmap.md`

## Chunk 1: Core Launch Semantics

### Task 1: Core force-place behavior

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/launch.rs`

- [x] **Step 1: Write failing core tests**

Add tests to `crates/porthole-core/src/launch.rs`:
- `force_place_applies_placement_on_preexisting`
- `force_place_without_effective_placement_is_not_requested`
- `require_fresh_and_force_place_is_invalid`

- [x] **Step 2: Verify red**

Run:

```bash
cargo test -p porthole-core launch::tests::force_place --locked
```

Expected: compile/test failure because `force_place` does not exist.

- [x] **Step 3: Implement core fields and behavior**

Add `force_place: bool` to `ProcessLaunchSpec` and `ArtifactLaunchSpec`, plus `LaunchSpec::force_place()`.

In `LaunchPipeline::launch`:
- before adapter dispatch, reject `require_fresh_surface && force_place` with `invalid_argument`
- after correlation, if `surface_was_preexisting`:
  - non-empty placement + `force_place == true` calls `apply_placement`
  - non-empty placement + `force_place == false` returns `SkippedPreexisting`
  - no effective placement returns `NotRequested`

- [x] **Step 4: Verify green**

Run:

```bash
cargo test -p porthole-core launch::tests::force_place --locked
```

Expected: PASS.

## Chunk 2: Wire And Routes

### Task 2: Protocol and daemon route coverage

**Files:**
- Modify: `crates/porthole-protocol/src/launches.rs`
- Modify: `crates/portholed/src/routes/launches.rs`
- Modify: `crates/portholed/src/routes/replace.rs`
- Modify: `crates/portholed/src/server.rs`

- [x] **Step 1: Write failing protocol/route tests**

Add protocol tests:
- missing `force_place` defaults false
- `force_place: true` round-trips

Add daemon tests in `crates/portholed/src/server.rs`:
- launch with preexisting scripted outcome, placement, and `force_place: true` returns `placement: applied`
- launch with both `require_fresh_surface` and `force_place` returns `invalid_argument`
- replace forwards `force_place` into the core launch spec

- [x] **Step 2: Verify red**

Run:

```bash
cargo test -p porthole-protocol launches::tests::launch_request_force_place --locked
cargo test -p portholed server::tests::post_launches_force_place --locked
```

Expected: FAIL because the wire field is absent/not forwarded.

- [x] **Step 3: Implement wire fields and forwarding**

Add `force_place` to `LaunchRequest` with serde default/skip-false behavior. Forward it into both process and artifact specs in launch and replace routes.

- [x] **Step 4: Verify green**

Run:

```bash
cargo test -p porthole-protocol launches::tests::launch_request_force_place --locked
cargo test -p portholed server::tests::post_launches_force_place --locked
```

Expected: PASS.

## Chunk 3: CLI And Roadmap

### Task 3: CLI surface and docs

**Files:**
- Modify: `crates/porthole/src/commands/launch.rs`
- Modify: `crates/porthole/src/main.rs`
- Modify: `docs/roadmap.md`

- [x] **Step 1: Write failing CLI tests**

Add CLI parser/command tests proving:
- `porthole launch --force-place ...` sets request `force_place: true`
- `porthole replace --force-place ...` sets request `force_place: true`

- [x] **Step 2: Verify red**

Run:

```bash
cargo test -p porthole force_place --locked
```

Expected: FAIL because the flag does not exist.

- [x] **Step 3: Implement CLI flag and roadmap update**

Add `--force-place` to launch and replace commands. Pass it through to `LaunchArgs` and `LaunchRequest`. Mark the roadmap product slice complete.

- [x] **Step 4: Run focused verification**

Run:

```bash
cargo test -p porthole force_place --locked
cargo test -p porthole-core launch::tests::force_place --locked
cargo test -p porthole-protocol launches::tests::launch_request_force_place --locked
cargo test -p portholed server::tests::post_launches_force_place --locked
```

Expected: PASS.

## Chunk 4: Full Verification

### Task 4: Required gates and commit

- [x] **Step 1: Run full gates**

Run:

```bash
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all commands exit 0.

- [x] **Step 2: Commit**

```bash
git add crates/porthole-core crates/porthole-protocol crates/portholed crates/porthole docs/roadmap.md docs/superpowers/plans/2026-05-18-launch-force-place-preexisting.md
git commit -m "Add force-place launch placement option"
```
