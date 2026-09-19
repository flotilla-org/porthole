# Porthole owns bridge execution; in-process is the default

Decision, 2026-09-19.

The cross-host bridge implementation belongs in Porthole. It consumes Jackstay's
transport and graph libraries; Jackstay must remain usable without Porthole.
Moving the bridge does not move desktop authority into Jackstay.

Exports and republications run inside `portholed` by default. Each bridge half
has an owned task with status, cancellation and teardown. Cancellation interrupts
blocked link I/O and joins the task's threads. Early errors must use the same
cleanup path as an explicit close; process exit is no longer the cleanup plan.

Separate workers remain an explicit per-operation choice (`execution: "worker"`
in the request, `--worker` in the CLI). They use the same bridge library. The
bundle includes the worker executable by default so this mode does not depend
on a separately installed tool.

On macOS, in-process republications use the daemon's existing Mach service.
Jackstay routes named-service connections to host-registered publications by
attach token. A connection stays with its selected publication, and retiring
one registration must not disturb the others or let that connection attach to
a replacement. This routing mechanism remains in Jackstay; the host supplies
the publications and capabilities.

In-process operation means a fatal codec or native-library fault can terminate
the daemon. Worker mode is available when process isolation is useful. The
choice of execution mode does not change permission checks, stream identity,
codec policy, or the wire protocol.
