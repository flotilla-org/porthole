# macOS Platform Bundle Foundation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the shell-owned macOS dev bundle assembly with a repo-native `cargo xtask bundle --platform macos` path that builds the same transitional `Porthole.app` from checked-in bundle metadata.

**Architecture:** Add an `xtask` crate for product assembly and a Cargo alias so `cargo xtask ...` works. Move macOS bundle metadata and resources under `apps/macos/bundle/`, teach xtask to assemble/sign `target/<profile>/Porthole.app`, and make `scripts/dev-bundle.sh` a compatibility wrapper around xtask. Keep `CFBundleExecutable=portholed` until the Swift helper lands.

**Tech Stack:** Rust `xtask`, Cargo workspace alias, checked-in macOS `Info.plist`, shell compatibility wrapper, existing `scripts/tests/test-dev-bundle.sh`.

---

## Scope Notes

This plan implements only the first slice from `docs/superpowers/specs/2026-05-17-platform-ui-apps-bundle-design.md`.

Do not add Swift helper UI here. Do not flip `CFBundleExecutable` to `PortholeHelper` in this slice. Do not change the bundle id. The output bundle must remain functionally equivalent to today's daemon-only `Porthole.app`, just built by a better path.

## File Structure

- Create `.cargo/config.toml` - Cargo alias for `cargo xtask`.
- Modify `Cargo.toml` - add `crates/xtask` to workspace members.
- Create `crates/xtask/Cargo.toml` - tiny internal binary crate.
- Create `crates/xtask/src/main.rs` - argument parsing and top-level dispatch.
- Create `crates/xtask/src/macos_bundle.rs` - macOS bundle assembly, signing identity selection, plist copy, resources copy.
- Create `crates/xtask/tests/macos_bundle.rs` - focused tests for argument parsing and plist/content decisions that do not require signing.
- Create `apps/macos/bundle/Info.plist` - checked-in transitional plist template.
- Create `apps/macos/bundle/Resources/icon.png` - authoritative macOS bundle icon input copied from `assets/icon.png`.
- Modify `scripts/dev-bundle.sh` - delegate to `cargo xtask bundle --platform macos`.
- Modify `scripts/tests/test-dev-bundle.sh` - assert the xtask path and transitional plist contents.
- Modify `README.md` - replace direct `scripts/dev-bundle.sh` build instructions with `cargo xtask bundle --platform macos --release`, keeping the wrapper mention.
- Modify `docs/roadmap.md` - tick the bundle-builder item only after implementation passes gates.

## Chunk 1: xtask Skeleton And Alias

### Task 1: Add cargo alias and workspace member

**Files:**
- Create: `.cargo/config.toml`
- Modify: `Cargo.toml`
- Create: `crates/xtask/Cargo.toml`
- Create: `crates/xtask/src/main.rs`

- [ ] **Step 1: Add failing command check**

Run:

```sh
cargo xtask --help
```

Expected before implementation: Cargo reports no such command or no package.

- [ ] **Step 2: Add the Cargo alias**

Create `.cargo/config.toml`:

```toml
[alias]
xtask = "run -p xtask --"
```

- [ ] **Step 3: Add `crates/xtask` to the workspace**

In root `Cargo.toml`, add:

```toml
"crates/xtask",
```

to `workspace.members`.

- [ ] **Step 4: Create the xtask crate**

Create `crates/xtask/Cargo.toml`:

```toml
[package]
name = "xtask"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
clap = { workspace = true }
thiserror = { workspace = true }

[lints]
workspace = true
```

Create `crates/xtask/src/main.rs` with a minimal command tree:

```rust
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Bundle(BundleArgs),
}

#[derive(Debug, Parser)]
struct BundleArgs {
    #[arg(long, value_enum)]
    platform: Platform,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    refresh: bool,
    #[arg(long)]
    sign: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Platform {
    Macos,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Bundle(args) => {
            println!(
                "bundle platform={:?} profile={}",
                args.platform,
                if args.release { "release" } else { "debug" }
            );
        }
    }
}
```

- [ ] **Step 5: Verify alias works**

Run:

```sh
cargo xtask --help
cargo xtask bundle --platform macos
```

Expected: both commands exit 0; bundle command only prints the placeholder.

- [ ] **Step 6: Commit skeleton**

```sh
git add .cargo/config.toml Cargo.toml crates/xtask
git commit -m "build: add xtask entrypoint"
```

## Chunk 2: Checked-In macOS Bundle Inputs

### Task 2: Move macOS bundle metadata under `apps/macos`

**Files:**
- Create: `apps/macos/bundle/Info.plist`
- Create: `apps/macos/bundle/Resources/icon.png`
- Modify: `crates/xtask/src/main.rs`
- Create: `crates/xtask/src/macos_bundle.rs`
- Test: `crates/xtask/tests/macos_bundle.rs`

- [ ] **Step 1: Add tests for plist expectations**

Create `crates/xtask/tests/macos_bundle.rs`:

```rust
use std::fs;

#[test]
fn transitional_info_plist_keeps_daemon_executable() {
    let plist = fs::read_to_string("apps/macos/bundle/Info.plist").unwrap();
    assert!(plist.contains("<key>CFBundleIdentifier</key>"));
    assert!(plist.contains("<string>org.flotilla.porthole.dev</string>"));
    assert!(plist.contains("<key>CFBundleExecutable</key>"));
    assert!(plist.contains("<string>portholed</string>"));
    assert!(plist.contains("<key>LSBackgroundOnly</key>"));
    assert!(!plist.contains("PortholeHelper"));
}

#[test]
fn macos_bundle_icon_input_exists() {
    assert!(std::path::Path::new("apps/macos/bundle/Resources/icon.png").is_file());
}
```

Run:

```sh
cargo test -p xtask --test macos_bundle --locked
```

Expected: FAIL because files do not exist.

- [ ] **Step 2: Add checked-in `Info.plist`**

Create `apps/macos/bundle/Info.plist` with the same transitional values currently emitted by `scripts/dev-bundle.sh`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>org.flotilla.porthole.dev</string>
    <key>CFBundleName</key>
    <string>Porthole</string>
    <key>CFBundleExecutable</key>
    <string>portholed</string>
    <key>CFBundleIconFile</key>
    <string>icon</string>
    <key>CFBundleVersion</key>
    <string>0.0.0-dev</string>
    <key>CFBundleShortVersionString</key>
    <string>0.0.0-dev</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSBackgroundOnly</key>
    <true/>
    <key>NSAccessibilityUsageDescription</key>
    <string>Porthole needs Accessibility to inject input and inspect window state.</string>
    <key>NSScreenCaptureUsageDescription</key>
    <string>Porthole needs Screen Recording to capture window screenshots and detect frame changes.</string>
</dict>
</plist>
```

- [ ] **Step 3: Move or copy the icon input**

Copy `assets/icon.png` to `apps/macos/bundle/Resources/icon.png`.

Keep `assets/icon.png` for this slice if other code or docs reference it; remove it only in a later cleanup if `rg assets/icon.png` proves no references remain.

- [ ] **Step 4: Add macOS bundle module stub**

Create `crates/xtask/src/macos_bundle.rs` with path constants:

```rust
use std::path::{Path, PathBuf};

pub const INFO_PLIST: &str = "apps/macos/bundle/Info.plist";
pub const ICON: &str = "apps/macos/bundle/Resources/icon.png";

pub fn app_path(profile: &str) -> PathBuf {
    Path::new("target").join(profile).join("Porthole.app")
}
```

In `main.rs`, add `mod macos_bundle;`.

- [ ] **Step 5: Verify tests**

Run:

```sh
cargo test -p xtask --test macos_bundle --locked
```

Expected: PASS.

- [ ] **Step 6: Commit bundle inputs**

```sh
git add apps/macos/bundle crates/xtask
git commit -m "build: add macOS bundle metadata"
```

## Chunk 3: Bundle Assembly

### Task 3: Implement `cargo xtask bundle --platform macos`

**Files:**
- Modify: `crates/xtask/Cargo.toml`
- Modify: `crates/xtask/src/main.rs`
- Modify: `crates/xtask/src/macos_bundle.rs`
- Test: `crates/xtask/tests/macos_bundle.rs`

- [ ] **Step 1: Add unit tests for command construction**

Extend `crates/xtask/tests/macos_bundle.rs` with tests for pure helpers:

```rust
use xtask::macos_bundle::{build_command_args, profile_name};

#[test]
fn profile_name_defaults_to_debug() {
    assert_eq!(profile_name(false), "debug");
    assert_eq!(profile_name(true), "release");
}

#[test]
fn build_command_for_debug_workspace() {
    assert_eq!(build_command_args(false), vec!["build", "--workspace"]);
}

#[test]
fn build_command_for_release_workspace() {
    assert_eq!(build_command_args(true), vec!["build", "--workspace", "--release"]);
}
```

This requires making `xtask` a library too:

```toml
[lib]
name = "xtask"
path = "src/lib.rs"
```

Run:

```sh
cargo test -p xtask --test macos_bundle --locked
```

Expected: FAIL until helpers exist.

- [ ] **Step 2: Split xtask into lib + bin**

Create `crates/xtask/src/lib.rs`:

```rust
pub mod macos_bundle;
```

Update `main.rs` to use `xtask::macos_bundle`.

- [ ] **Step 3: Implement pure helpers**

In `macos_bundle.rs`:

```rust
pub fn profile_name(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

pub fn build_command_args(release: bool) -> Vec<&'static str> {
    if release {
        vec!["build", "--workspace", "--release"]
    } else {
        vec!["build", "--workspace"]
    }
}
```

Run the focused test again and verify PASS.

- [ ] **Step 4: Implement signing identity selection**

Port the existing `choose_sign_identity` behavior from `scripts/dev-bundle.sh` into Rust:

- If `--sign` is provided, use it.
- Otherwise run `security find-identity -v -p codesigning`.
- Select the first identity whose name starts with `Apple Development:`.
- If none is found, return the existing explanatory error text.

Keep this logic in a small function so tests can cover parsing without invoking `security`:

```rust
pub fn parse_apple_development_identity(output: &str) -> Option<String> {
    output.lines().find_map(...)
}
```

Add tests for parsing a sample identity output and for ignoring ad-hoc signatures.

- [ ] **Step 5: Implement bundle assembly**

`cargo xtask bundle --platform macos` should:

1. Build Rust workspace unless `--refresh`.
2. Check `target/<profile>/portholed` and `target/<profile>/porthole` exist.
3. Remove and recreate:
   - `Porthole.app/Contents/MacOS`
   - `Porthole.app/Contents/Resources`
4. Copy:
   - `apps/macos/bundle/Info.plist` to `Contents/Info.plist`
   - `apps/macos/bundle/Resources/icon.png` to `Contents/Resources/icon.png`
   - `target/<profile>/portholed` to `Contents/MacOS/portholed`
   - `target/<profile>/porthole` to `Contents/MacOS/porthole`
5. `chmod +x` the two binaries.
6. Run `codesign -s <identity> --force --deep <app>`.
7. Print the same install/onboard guidance as today.

Use `std::process::Command` and return typed errors with context. Do not use shell command strings for build/sign operations.

- [ ] **Step 6: Verify focused command**

Run:

```sh
cargo xtask bundle --platform macos
```

Expected with signing identity: builds and signs `target/debug/Porthole.app`.

Expected without signing identity: exits nonzero with "Apple Development signing identity required for Porthole dev bundles."

- [ ] **Step 7: Verify app contents**

Run:

```sh
codesign -v target/debug/Porthole.app
test -x target/debug/Porthole.app/Contents/MacOS/portholed
test -x target/debug/Porthole.app/Contents/MacOS/porthole
/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' target/debug/Porthole.app/Contents/Info.plist
```

Expected `PlistBuddy` output: `portholed`.

- [ ] **Step 8: Commit xtask bundle assembly**

```sh
git add Cargo.toml crates/xtask apps/macos/bundle
git commit -m "build: assemble macOS bundle with xtask"
```

## Chunk 4: Wrapper, Tests, And Docs

### Task 4: Delegate `scripts/dev-bundle.sh` to xtask

**Files:**
- Modify: `scripts/dev-bundle.sh`
- Modify: `scripts/tests/test-dev-bundle.sh`
- Modify: `README.md`
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Rewrite wrapper**

Replace the internals of `scripts/dev-bundle.sh` with argument translation:

```bash
#!/usr/bin/env bash
set -euo pipefail

args=(bundle --platform macos)
while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) args+=(--release); shift ;;
        --refresh) args+=(--refresh); shift ;;
        --sign)
            if [[ $# -lt 2 ]]; then
                echo "--sign requires an identity" >&2
                exit 1
            fi
            args+=(--sign "$2")
            shift 2
            ;;
        --adhoc)
            echo "--adhoc is not supported for Porthole dev bundles." >&2
            echo "Ad-hoc signatures change designated requirement on rebuild and invalidate TCC grants." >&2
            exit 1
            ;;
        -h|--help)
            cargo xtask bundle --help
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

exec cargo xtask "${args[@]}"
```

- [ ] **Step 2: Update bundle smoke test**

In `scripts/tests/test-dev-bundle.sh`, keep the `--adhoc` rejection test and then call:

```sh
cargo xtask bundle --platform macos
```

instead of direct shell assembly. Add:

```sh
exec_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' target/debug/Porthole.app/Contents/Info.plist)"
test "$exec_name" = "portholed" || { echo "expected transitional executable portholed, got $exec_name" >&2; exit 1; }
```

- [ ] **Step 3: Update README**

In `README.md`, replace:

```sh
./scripts/dev-bundle.sh --release
```

with:

```sh
cargo xtask bundle --platform macos --release
```

Mention that `scripts/dev-bundle.sh` remains a compatibility wrapper.

- [ ] **Step 4: Update roadmap**

Tick:

```markdown
- [x] Repo-native bundle builder (`cargo xtask bundle --platform macos`) ...
```

Do not tick the Swift helper item.

- [ ] **Step 5: Run wrapper smoke**

Run:

```sh
./scripts/dev-bundle.sh --refresh
```

Expected: delegates to xtask and signs the existing bundle if binaries exist.

- [ ] **Step 6: Run script test**

Run:

```sh
./scripts/tests/test-dev-bundle.sh
```

Expected: PASS if an Apple Development identity is present; otherwise PASS by verifying the explicit missing-identity error path.

- [ ] **Step 7: Commit wrapper and docs**

```sh
git add scripts/dev-bundle.sh scripts/tests/test-dev-bundle.sh README.md docs/roadmap.md
git commit -m "docs: route macOS bundle workflow through xtask"
```

## Chunk 5: Final Verification

### Task 5: Run gates

**Files:**
- No new files unless fixes are required.

- [ ] **Step 1: Run focused xtask tests**

```sh
cargo test -p xtask --locked
```

- [ ] **Step 2: Run bundle smoke**

```sh
./scripts/tests/test-dev-bundle.sh
```

- [ ] **Step 3: Run workspace gates**

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

- [ ] **Step 4: Inspect bundle identity**

If an Apple Development identity is available, run:

```sh
codesign -dv target/debug/Porthole.app 2>&1 | sed -n 's/^Identifier=//p; s/^Authority=//p'
/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' target/debug/Porthole.app/Contents/Info.plist
/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' target/debug/Porthole.app/Contents/Info.plist
```

Expected:

- identifier/plist bundle id is `org.flotilla.porthole.dev`
- executable is still `portholed`
- signing authority is Apple Development, not ad-hoc

- [ ] **Step 5: Open PR**

Open a draft PR with:

- Summary: xtask bundle builder, checked-in macOS metadata, wrapper migration.
- Validation: all commands above and whether signing-dependent smoke ran or only missing-identity path ran.

Do not claim live macOS permission behavior was verified unless you actually ran an installed-bundle `porthole onboard`/recording smoke with permissions granted.
