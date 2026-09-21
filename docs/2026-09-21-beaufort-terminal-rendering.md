# Beaufort SSH attachment and terminal rendering

The operator reported that attachment, resize, detach and reattachment from
kiwi worked using the staged Cleat client from
[#221](https://github.com/flotilla-org/cleat/pull/221). Kiwi runs Ghostty.
The operator also observed flicker around Codex's animated stars above the
prompt, and a monochrome interface both remotely and in Beaufort's local console.
This is operator-reported SSH acceptance, not an agent-run remote recording.

## Confirmed color cause

The existing session reported `vt_engine: ghostty`,
`vt_engine_status: functional`, and `functional_vt_available: true`.
Ghostty VT was active throughout; switching backends was unnecessary.

A whitelist-only environment probe launched into the same Cleat daemon found
`TERM=xterm-256color`, `NO_COLOR=1`, and no `COLORTERM` or `TERM_PROGRAM`.
The automation harness also had `NO_COLOR=1`. Two otherwise identical idle
Codex sessions were then launched through the same daemon into the same trusted
workspace. Only the second child cleared `NO_COLOR` before invoking Codex.

Recorded native output contained zero RGB SGR codes in the first case and 87
distinct RGB SGR codes in the second. The existing agent's recording likewise
contained no RGB codes. This isolates the monochrome cause to the inherited
environment, independently of SSH or the outer terminal.

The vessel's `agent-entry.ps1` now clears `NO_COLOR` for the interactive Codex
process. Generic Cleat behavior is unchanged; terminal type remains supplied by
the VT backend. This is not a reason to override color preferences in unrelated
CLI commands. The test-owned vessel was restarted under the operator's standing
authorization. The previous identity was observed revoked first.

The new run is `C:\dev\windows-parity-plan\vessel-color`, with daemon/session
`beaufort-color/coding-agent`, identity
`agent_0f5b0bd2f05940daafe3ac394b718bdc`, wrapper PID 7320 and continuity marker
`cf818517-cf41-49a3-9983-46648de71f2f`. It emits RGB codes and passed the native
desktop proof at `2026-09-21T17:04:30.2235779Z`; the screenshot hash again matches
`261F7F4E45FAD52B3E7960B819BDA2D9618EB863A14CB9479773667045F7578A`.
This deliberate restart begins a new agent run; it is not a continuation claim
for the old identity.

## Flicker remains under investigation

A small native probe wrote a synchronized-update begin marker plus `SYNC_A`,
waited 100 ms, then wrote carriage return plus `SYNC_B` and the end marker.
The recorded ConPTY output put the end marker before the final `SYNC_B` text:

```text
output chunk 1: ESC[?2026h ... SYNC_A ...
output chunk 2: ESC[?2026l ... SYNC_B ...
```

That ordering is a concrete local redraw-boundary observation. It may contribute
to partial-frame presentation through Windows console layers, but it does not
establish the complete cause of the kiwi-side flicker. No terminal-buffering or
ConPTY-runtime change is claimed here. The colored session needs comparison on
kiwi before the visible flicker can be called resolved.

Diagnostic scripts and captures remain under the explicitly marked local
`C:\dev\windows-parity-plan\debug` directory. Temporary probe sessions were
stopped; the unrelated RAD daemon was preserved. No token values were printed.

## Reconnect to the colored run

```bash
ssh -t rober@beaufort 'C:\dev\windows-parity-plan\cleat-safe-attach-bin\cleat.exe --runtime-root C:\dev\windows-parity-plan\vessel-color\cleat-state --server beaufort-color attach coding-agent --no-create --identity kiwi --take'
```

Detach with Ctrl-] then d. The previous command names the previous, now stopped
run and should fail rather than start a replacement. RDP/lock continuity and
login startup registration remain outstanding.
