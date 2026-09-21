# Windows Cleat attachment route: initial findings

Date: 2026-09-21. Research for [Establish a safe kiwi-to-Windows Cleat attachment route](https://github.com/flotilla-org/porthole/issues/152).

## Finding

Cleat has a native Windows daemon transport and ConPTY backend, but the current CLI is not a safe client for an externally owned remote endpoint. The smallest sound candidate is a **connect-only attachment path to an explicitly selected existing daemon/session**, carried by a bounded SSH byte-stream adapter that opens the Windows named pipe. This is a proposed implementation packet, not a working command or an accepted transport decision. Full Tender is not required.

The user requires the same agent and terminal to survive attachment detach, RDP disconnect and Windows lock. Desktop actions may wait or fail clearly while the desktop is unavailable. Recreating a terminal with the same name does not meet that requirement.

## Evidence and limits

Inspected the clean local Cleat checkout at `d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc` and Flotilla documentation at `906ecb36885efe0262f267f30c7a2a0073f59ff4`. Read repository instructions, including Flotilla's AGENTS-to-CLAUDE reference, without applying Flotilla workflow rules to this Porthole map. Queried current issue bodies. No daemon, session, RDP, SSH connection or transport helper was started, stopped or modified. No tests were run; this report changes documentation only.

The [attach and Windows follow-up](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/docs/attach-and-windows-fixes-2026-09.md) records real gouda Windows builds, provider tests, Ghostty library/VT tests and normal setup-helper validation. It also records an intermittent full CLI integration failure with pipe-ended error 109. These are previous recorded results, not live Beaufort acceptance. [Add Windows CI coverage for platform-specific session paths](https://github.com/flotilla-org/cleat/issues/169) remains open, but its original minimum is partly superseded by the Windows Rust-only build/library gate. [Extend Windows coverage to Ghostty builds and provider reconnect behavior](https://github.com/flotilla-org/cleat/issues/205) explicitly identifies remaining Ghostty CI and Windows reconnect-test gaps. Do not infer that all Windows behavior is unimplemented, or that core CI proves remote attachment.

## Implemented endpoint and ownership behavior

| Area | Observed source behavior | Consequence |
| --- | --- | --- |
| Local Windows IPC | `platform/ipc/windows.rs` implements duplex byte named pipes with overlapped reads/writes. A filesystem `socket` marker contains the pipe name; the pipe name is derived from the marker path. | This is real local byte-stream support. The marker is not a Unix socket and forwarding its pathname does not reach the pipe. |
| Runtime layout | `runtime.rs` defines root/daemon-name/session directories, socket marker, daemon PID and foreground files. `CLEAT_RUNTIME_DIR` and daemon selection identify local runtime state. | A remote exposure must not masquerade as locally owned runtime state. A local PID is not proof of a remote daemon's identity. |
| List | `server.rs::daemon_names` and `list_one_daemon_with_selectors` require a local sessions directory; list calls `ensure_daemon_started` before requesting `GET /sessions`. | A socket-only exposure can return an empty list or trigger local startup. |
| Startup | `session.rs::ensure_daemon_started` returns only if endpoint connection succeeds **and** local PID liveness succeeds. Otherwise it can remove a socket marker, create runtime directories and spawn a local daemon. | Even a connectable remote endpoint is insufficient without local ownership facts. A failed route must not enter this path. |
| Attach | `SessionService::attach --no-create` first checks local session files. Failed inspect can remove the local session directory unless recreatable state exists. Ordinary attach can ensure/create/recreate a session. | `--no-create` alone is not a connect-only ownership contract. |
| Control | Inspect, keys, resize and many other verbs retain local session-directory gates before protocol calls. Owner resolution enumerates directories and reads daemon PID registration. | Protocol capability and CLI endpoint independence are different things. |
| Capture | Current screen capture uses daemon `GET /sessions/{id}/screen`, after local gate/startup. Historical capture slices read the local cast file. | The old issue's broad capture warning needs this distinction: screen data is now protocol-served; transcript slices remain filesystem-dependent. |
| Cleanup | `sweep_dead_daemon_sessions` removes non-recreatable sessions, foreground state, socket and PID files when control is unavailable; kill has related fallback paths. | Route failure must not become authority to clean remote state or a forwarding listener. |

Sources: [Windows IPC](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/crates/cleat/src/platform/ipc/windows.rs), [runtime](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/crates/cleat/src/runtime.rs), [session service](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/crates/cleat/src/server.rs), [daemon startup and session loop](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/crates/cleat/src/session.rs).

These findings confirm the central concern in [CLI verbs gate on local runtime-root state — socket-only (forwarded) daemons break or trigger local shadow daemons](https://github.com/flotilla-org/cleat/issues/122), against current source rather than its older line numbers.

The packet provider is the better existing seam: `provider_daemon.rs::connect_packet_stream` opens the transport, performs `/connect` upgrade and reads the directory protocol. Its reader loop reconnects, refreshes the directory, reopens live channels and reasserts desired dimensions. Closed channels are skipped; old grants are discarded, and control is not silently stolen on reconnect. It still takes `RuntimeLayout`, so this is a reusable implementation seam, not evidence of an exposed remote-client option. Source: [packet daemon provider](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/crates/cleat/src/provider_daemon.rs).

The ConPTY backend owns the pseudoconsole independently of any attaching client's terminal, resizes it in character cells and closes it in `PtyChild::drop`. The daemon has separate attachment bookkeeping. This supports a persistence design, but does not prove RDP/session-policy behavior on Beaufort. Source: [Windows PTY backend](https://github.com/flotilla-org/cleat/blob/d5039f2dcd5ac25080cf6918d5e8b9ebe26f80bc/crates/cleat/src/platform/pty/windows.rs).

## Transport candidates

1. **SSH command with binary stdio-to-pipe adapter, plus explicit connect-only Cleat client:** preferred first implementation candidate. A non-PTY SSH command opens only the configured pipe and copies ordered bytes. The existing GUI-session daemon owns the terminal/agent; the SSH helper must never launch them. This avoids treating a Windows marker as a Unix socket and avoids a new listening TCP service. It requires actual adapter and client work: no such SSH pipe bridge was found in Cleat's CLI/platform IPC inspected here.
2. **SSH TCP forward to a Windows loopback-to-pipe adapter:** workable alternative in principle, but adds a listener and authorization boundary. Loopback reachability alone is not authorization to the terminal daemon. Specify listener ownership, access control, credentials, cancellation and cleanup before selecting it.
3. **SSH remote `cleat attach --no-create` in a PTY:** potentially a diagnostic route using server-side files, but not yet a sound recipe. Current inspect-failure cleanup, terminal nesting/resize and SSH account access to the GUI user's pipe need verification. It must not be advertised as already safe merely because no-create exists.
4. **Tender exposure:** a later implementation can supply the same endpoint contract. Its discovery, publication and identity system is not a prerequisite for this host-specific proof.

OpenSSH documents forwarding to TCP ports and Unix-domain sockets, not Win32 named-pipe adaptation. Its non-PTY command facility can carry an adapter's streams; that does not itself implement the adapter. Source: [OpenSSH ssh manual](https://man.openbsd.org/ssh).

Named-pipe access is checked against the server pipe's security descriptor. Current Cleat pipe creation passes a null security-attributes pointer and therefore relies on default security; this report has not inspected Beaufort's effective DACL or established that an SSH logon token can open it. The adapter must run under explicitly intended authority, and record actual success/denial rather than assuming the same account name is enough. Source: [Microsoft named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights) and Windows IPC source above.

[Tender's provisional architecture](https://github.com/flotilla-org/flotilla/blob/906ecb36885efe0262f267f30c7a2a0073f59ff4/docs/architecture/tender.md) and [ordinary-service forwarding research](https://github.com/flotilla-org/flotilla/blob/906ecb36885efe0262f267f30c7a2a0073f59ff4/docs/research/2026-09-13-tender-ordinary-service-forwarding-contracts.md) supply the appropriate boundary: applications own sessions and recovery, adapters own connections/listeners, broken streams are not replayed, and connection loss is not withdrawal or permission to replace the service. These are design documents, not implemented Tender guarantees.

## Candidate first work packet and acceptance

**Owner: Cleat.** Add an explicit externally owned endpoint selection for existing-session attach and necessary discovery/control. Separate endpoint access from local runtime ownership; prohibit spawn, recreation, local PID checks and runtime cleanup on that path. Keep cast-file-only commands explicitly unavailable there until they have a protocol implementation. Reuse packet directory/attach/reconnect machinery where possible. Test this contract with an injected endpoint, including failure after connect succeeds. This can build on the existing remote-runtime issue rather than creating a competing umbrella.

**Adapter owner: a small Cleat-adjacent transport component initially; exact repository home remains a map decision.** Add the SSH command/pipe byte bridge only after its boundary is agreed. Fixed approved endpoint selection, binary stdin/stdout with diagnostics only on stderr, bounded buffers, cancellation and prompt full closure are minimum requirements. Decide directional EOF behavior explicitly; do not assume socket half-close maps to the current pipe wrapper. Never replay input on reconnect. If a generic transport package already emerges from Tender work, reuse it without waiting for the full service.

**Beaufort orchestration/evidence owner: Porthole Windows parity effort, linked Cleat changes.** Prepare a separate named daemon in the existing GUI login through the Porthole launch recipe. Leave RAD and its runtime tree untouched. Record executable revisions, exact runtime root, daemon name, daemon PID, agent PID/start time, Windows session ID and a terminal continuity nonce before connecting.

Required acceptance evidence:

- With no local sessions directory or daemon PID, kiwi lists/attaches the prepared session through the external endpoint; input/output and resize work. Reconnect returns the same continuity nonce and process identity.
- Manual detach closes only the attachment. A later attach reaches the same terminal and agent. Record the before/after PIDs and start times, not just an identical display name.
- Break the route mid-stream; bounded failure reaches the client, no input is replayed, and the daemon/agent continue. Restore it and establish a fresh protocol connection/full render. Existing controller policy is respected.
- Wrong endpoint, unavailable pipe, permission denial, unavailable remote daemon and failed post-connect handshake all fail clearly. Assert no local daemon spawned, no runtime directory fabricated, no marker/PID/session files removed, and no replacement session created remotely.
- Kill only the adapter; prove it neither owns nor terminates the GUI daemon/ConPTY. Confirm no helper remains blocked forever on a pipe read after cancellation.
- RDP disconnect and Windows lock are separate cases. Preserve the same agent/terminal; attempt a desktop action and record bounded waiting or an explicit unavailable/failure result. On return to an unlocked usable GUI, verify terminal continuity and fresh visible input/screenshot evidence. No claim of unattended desktop control while locked is needed.
- Record Windows session policy relevant to disconnect/logoff; sign-out/reboot survival is outside this first continuity contract. A policy that logs off the GUI session would require an explicit deployment decision, not silent session recreation.

The earliest executable engineering packet is the Cleat external-endpoint ownership seam with negative tests. The first end-to-end Beaufort packet depends on that seam, an implemented pipe adapter and the independently prepared GUI-session daemon. The research resolves which gaps must be covered; it does not claim the preferred adapter or the deployment recipe has passed acceptance.
