# Windows native capture handed to Jackstay: evidence (#186)

Part of [#186](https://github.com/flotilla-org/porthole/issues/186), which
implements the capture placement decision in
[jackstay#23](https://github.com/flotilla-org/jackstay/issues/23): capture runs
inside portholed; Porthole authorizes and selects the window, creates its
`GraphicsCaptureItem` and hands it to Jackstay with policy parameters; Porthole
keeps lifecycle policy. The design is in [windows-desktop.md](windows-desktop.md#native-capture-sessions).

## Host and revision

- Beaufort, Windows 11 Pro 10.0.26200, RDP session 1 (`rdp-tcp#0`, active,
  unlocked throughout), Rust 1.98.1 MSVC.
- Porthole `dda211f` (branch `windows/native-capture`), Jackstay `a01ea60`.
- Producer device: adapter `00000000:00009fe5`, AMD Radeon(TM) Graphics
  (Jackstay's default adapter here).

Everything below ran against a separate, isolated daemon
(`examples/windows_capture_server`, pipe `\\.\pipe\porthole-p1cap-<random>`,
in-memory policy store) and a temporary agent identity with grants approved
only for it, then revoked. Only windows created for the test were captured.
The user's own daemon, Cleat daemons and windows were not touched. No
whole-screen capture was used.

## Observed

### Live daemon, separate consumer process

`scripts/windows-native-capture-smoke.ps1`; logs (attach tokens redacted) are
in [evidence/windows-native-capture-20260925](evidence/windows-native-capture-20260925/).

| Scenario | Observed |
| --- | --- |
| Authorization | `launch` and `capture-session surface --native` each returned `agent_permission_needed` until the operator approved the temporary identity's request; only that identity's requests were approved. The capture grant was `observe` + `record` on that one surface; launch was `manage` on `launched_by_agent`. |
| Start | Session `ready`, transport 3 (Local Endpoint), 320×200, `publication=d3d11 on adapter 00000000:00009fe5`, `border=hidden`. |
| Wrong attach token | A consumer with a wrong token was refused before any Jackstay setup: `attach request is not authorized for this capture session`. |
| Separate consumer receives D3D11 frames | `windows_native_capture_consumer` (its own process) described the producer, created its device on the producer's LUID, attached, opened the shared textures and fence, GPU-waited and read back: 24 frames verified as the fixture's solid colour. |
| Resize | The fixture resized its client area to 480×300. The consumer installed replacement pools (generation 2, then 3) and verified 480×300 frames; `capture-session status` reported `ready 480x300`, same epoch. One frame (321×201, generation 2) was not uniform: WGC's first frame at the new size, before the window repainted. The consumer logs such a frame, within 250 ms of a new generation, as transitional. |
| Window close | `porthole close` closed the fixture. The session became `failed`: `captured window closed (closed: window destroyed (IsWindow))`. The consumer saw `publication closed`, and its reattach was refused with the same reason. Resources then drained (`native resources retired`). |
| Cleanup | Session closed, identity revoked, isolated daemon stopped; no fixture, consumer or daemon process remained. |

### Injected lock, RDP disconnect and device loss

`cargo test -p portholed --lib native_session_windows -- --ignored` runs
`native_capture_follows_lock_disconnect_device_loss_resize_and_close`. It
creates its own 240×160 window and starts a session through the same `create`
path, with Jackstay's `DesktopMonitor` injected and device loss triggered by
`WgcCapture::simulate_device_loss`. A consumer attaches over the real Local
Endpoint. It passed 7 consecutive runs.

| Scenario | Observed |
| --- | --- |
| Lock (injected `Locked`) | `paused`, `desktop unavailable: session locked`. A repaint while paused was not published and the published count did not move. After the lock cleared: `ready`, same epoch, frames published again. |
| RDP disconnect (injected `Disconnected`) | `paused`, `desktop unavailable: session disconnected`; the same behaviour as lock, then `ready`. |
| Device loss (simulated) | `ready` with `epoch=2`. The epoch-1 consumer saw its publication close. A new attach got epoch 2, and its frames verified. |
| Resize after recovery | 320×200 frames on the new epoch; status width 320. |
| Window close | `failed`, `captured window closed`; a new attach was refused with that reason; resources drained after the consumer left. |

After a pause ends, a window that does not repaint stays stale: Jackstay drops
frames that arrive while paused and keeps no copy to publish on resume (its
deferred-frame copy covers only capacity drops). The test therefore repaints
after resume. It is not yet known whether a real unlock or reconnect makes WGC
deliver a fresh frame; the human check below records it.

## Not observed: real lock and RDP disconnect

Nobody locked the workstation or disconnected RDP: a person is using Beaufort.
Pausing, resuming and epoch recovery are implemented in Jackstay and mapped by
Porthole, but they are only tested with injected conditions. Jackstay's own
real-WGC watch ([acquisition-d3d11.md](https://github.com/flotilla-org/jackstay/blob/a01ea606b3fb0eac93eddb5c7cf2e0ab4ffbe520/docs/design/acquisition-d3d11.md#lock-and-rdp-disconnect-not-yet-observed))
is also pending.

### Human-coordinated check

Run this in the RDP session that will be locked or disconnected, from a
Windows PowerShell in a checkout of this branch. It uses its own daemon and
pipe, and a test window; it does not disturb the user's daemon.

```powershell
cargo build -p porthole --locked
cargo build -p portholed --examples --locked
cargo build -p porthole-adapter-windows --example capture_fixture --locked
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows-native-capture-smoke.ps1 `
  -EvidenceDir target\native-capture-human -WatchSeconds 600
```

When the script logs `watching for 600 s`, the small fixture window (top left,
changing colour every second) is being captured and a consumer process is
verifying frames. Then:

1. **Lock.** Press Win+L, wait 30 s, then unlock.
   - Expected in `status-watch.log`: `paused ...: desktop unavailable: session
     locked ... epoch=1`, then `ready ... epoch=1` within a second of
     unlocking. The epoch does not change. `quser` shows the session `Active`.
   - Expected in `consumer.log`: no new frames while locked, then
     `uniform <colour>, verified` frames again after unlocking, with the same
     epoch.
2. **RDP disconnect.** Disconnect the RDP client (close it, or choose
   Disconnect; do not sign out). Wait 60 s, then reconnect from the same
   client.
   - Expected in `status-watch.log`: `paused ...: desktop unavailable: session
     disconnected` (`quser` shows `Disc`), then either `ready ... epoch=1`, or
     `recovering ...: device lost: ...` followed by `ready ... epoch=2`.
     Record the adapter in `publication=d3d11 on adapter ...` before and after.
   - Expected in `consumer.log`: frames verified again after reconnecting. With
     a new epoch, `publication closed`, then `attached: publication D3d11,
     epoch 2`, and verified frames.
3. Optionally, reconnect at another resolution or from another client to force
   an adapter change, and repeat step 2.
4. Let the script finish. It closes the fixture and expects `failed: captured
   window closed`, then revokes its identity and stops its daemon. Attach
   `commands.txt`, `status-watch.log` and `consumer.log` to #186 after
   redacting `ptas_...` tokens.

Any of these counts as a failure: a `failed` or `recovery_required` state
before the fixture closes; `NOT uniform` frames after resuming (other than a
`transitional` frame straight after a new pool generation); no `ready` within
10 s of unlock or reconnect; or the consumer not verifying frames again. Also
record whether WGC delivered a frame on resume without a repaint (the fixture
repaints every second, so check the first `verified` timestamp after resume).
For raw WGC behaviour without Porthole, Jackstay's
`cargo run -p jackstay --features backend-windows --example wgc_session_watch`
logs the same transitions.

## Gates

On Beaufort: `cargo build --workspace --locked`, `cargo test --workspace
--locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and
`cargo +nightly-2026-03-12 fmt --check` pass. macOS and Linux are checked by
CI. They could not be cross-checked here: Jackstay's build script needs their
native toolchains.
