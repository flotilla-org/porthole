# Porthole Darwin fleet delivery

The first adoption targets are comte and kiwi. Porthole gets its own
`porthole-fleet` generations. Each generation records its Porthole and Jackstay
commits, Jackstay C ABI, and bridge wire version. Katzensteg can release
independently; compatibility does not require identical Jackstay commits.

## Candidate build

`.forgejo/workflows/fleet-candidate.yml` runs only on manual dispatch, with an
exact Porthole commit. The workflow fetches that public GitHub commit and runs
on the existing `darwin-aarch64` worker. GitHub continues to run PR checks.
The Forgejo mirror and the VM controller's repository allowlist must include
Porthole before dispatch will work.

The candidate builder checks that the checkout is clean, resolves Jackstay
through locked Cargo metadata, and requires both Jackstay packages to use the
same exact canonical commit. It builds the bridge with `backend-macos` and
packages these files:

```text
porthole-candidate-darwin-aarch64/
  candidate.json
  Porthole.app/Contents/
    Info.plist
    MacOS/{PortholeHelper,portholed,porthole,jackstay-bridge}
    Resources/icon.png
    Library/LaunchAgents/work.flotilla.porthole.daemon.plist
```

The manifest records every payload's digest, size, and mode. The adjacent JSON
records the archive digest and size. Neither file is a signing attestation.
The builder requires arm64 executables with only system dynamic-library
dependencies and rejects build-directory runtime search paths. The SDL
reference viewer remains a separate developer tool; it is not currently part
of `Porthole.app`.

SwiftBuild can insert an absolute fallback search path to the active Xcode
toolchain's Swift libraries. The candidate builder removes that specific
fallback before recording digests, then checks all runtime linkage. Other
nonportable search paths remain build errors. Central signing covers the
resulting bytes.

Cargo dependencies and release outputs are cached. Swift's scratch directory
and Clang/Swift module caches are fresh per build to avoid stale serialized
module paths from another checkout. Toolchain versions are recorded in the
manifest. The runner image controls installed toolchains; this does not claim
bit-for-bit reproducibility across image updates.

For a clean checkout on a worker without signing identities:

```sh
python3 scripts/fleet/build_candidate.py \
  --source-sha "$(git rev-parse HEAD)" \
  --output /path/to/new-output-directory
```

Outputs are retained as run artifacts for seven days. They have state
`requires-central-signing` and cannot be offered as an installable generation.
Apple toolchains can place ad-hoc signatures on linked executables; the
candidate makes no claim of a trusted release signature. The build job holds
no release credential and requires zero signing identities.

## Central signing and promotion still required

The current `lab-darwin-sign` accepts Flotilla/Cleat's fixed payload shape.
It needs a reviewed Porthole adapter before this candidate can be signed.
Keep the established Comte identity and team policy, digest-verified input,
fixed-function signing, derivative archive, and authenticated attestation.
The signer must not execute Porthole's build scripts or candidate binaries.

The adapter must validate the candidate schema and exact file set, reject
archive traversal and links, verify all payload digests, and check the bundle
identifier and LaunchAgent fields against trusted constants. It signs the
four executables individually, then seals the outer app, including the
LaunchAgent plist. It verifies nested signatures, team, entitlements, and
designated requirements before emitting an attested derivative. Signing with
`--deep` is unnecessary. Existing development bundle identifiers must remain
stable so that certificate-backed TCC requirements can be checked during
adoption.

Promotion publishes immutable `lab/porthole-fleet` versions after validating
the signed derivative and its source linkage. The completion marker must be
written last. Reuse the fleet transport, digest, attestation, and immutable
publication primitives where their interfaces permit it. The current
Flotilla-specific generation schema is not the Porthole candidate schema.

## Pull installation and rollback still required

The proposed command is `porthole-fleet-install`, shipped by Porthole and
bootstrapped through the same reviewed-tool process as `fleet-install`.
It should support `status`, `latest`, an explicit generation, and `rollback`.
Pulls remain operator initiated. First adoption must check the existing
bundle location and supervision state on each host before replacing anything.

Archive generations can live under `~/.local/opt/porthole-fleet`, but activation
must install a real app at the host's selected Applications path. A symlink to
a generation is not enough to establish SMAppService registration. The
existing `porthole install --force` deletes the previous bundle before copying
and has no rollback or registration transaction; it is not yet the fleet
activation implementation.

Activation needs a staged, signature-verified app on the same volume, retained
previous bytes, and an atomic replacement at the stable app path. Registration
and daemon health cannot be made atomic with the filesystem exchange, so use
a recoverable transaction: unregister/stop the old helper-managed service,
replace the app, register through the new helper, verify the selected daemon,
then commit the generation record. On failure, restore the previous app and
its registration. Interrupted transactions need explicit recovery on the next
invocation. Preserve a deliberately disabled development daemon state.

The helper currently registers services at startup but exposes no transactional
unregister/register command for the installer. That interface and tests belong
in the activation change. macOS can still require human Login Items or TCC
approval; report the exact missing permission and wait for it.

Before calling the path complete, prove on comte and kiwi: install, daemon
registration, capture/input permissions, bridge startup, upgrade, failed
activation recovery, and rollback. Record app location, designated requirement,
source generation, and observed daemon identity. No host has been switched by
the candidate-build change.
