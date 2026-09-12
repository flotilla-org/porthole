# CPU capture acquisition integration

The CPU capture host, recorder and Jackstay SDL viewer use the common arena.
Porthole pins Jackstay `f482a5ca1c0c0a3831d3b4774dc8b7110c4581eb` (C ABI 0.5).
This replaces per-frame socket requests, connection lease IDs and shadow-ring
validation with setup followed by shared acquisition. A successful frame keeps
its bytes and descriptor until release, including across history wrap, consumer
destruction and setup disconnect.

Porthole still authorizes each session. The CPU socket accepts a bounded
`open_cpu_acquisition` preface containing session ID, track ID and bearer token.
After authorization it hands the connection to Jackstay's process-bound setup.
A restarted program gets a fresh incarnation. Old claims and mappings continue
to count until their retirement is established.

Each session has eight resources, two history entries, one producer reserve,
at most four incarnations and a 512 MiB allocation budget. Replacement includes
the cost of old allocations. Insufficient overlap capacity pauses publication
and drops incoming frames. Maintenance advances the pending transition when
storage becomes available, including when capture is idle.

Close stops publication and acquisition and shuts down setup connections. The
session remains queryable while a retained owner waits for claims, mappings and
setup owners to retire. Five seconds without that proof reports recovery
required; it does not free the storage. Startup cancellation aborts the capture
task, and status preserves a source failure after retirement. Status describes
the installed format while replacement is pending.

The recorder reserves one hold, releases after the writer consumes the bytes,
and waits using a snapshot taken before selection. It retains strict and
best-effort handling of ordered gaps. A configuration change ends the recording
with an explicit error because the movie writer uses fixed settings. The viewer
reserves two holds, copies into an SDL texture before release, and handles
configuration replacement through the common C API.

Focused tests cover authorization, history wrap, independent duplicate holds,
restart, paused replacement, startup cancellation, failure status and recorder
waits. `cpu_viewer_e2e` runs a child viewer presenting 60 synthetic frames,
then a fresh process presenting eight frames with a 250 ms hold per frame while
the host keeps publishing. The viewer checks held bytes before and after each
delay. The test then checks host retirement. Run it after building Jackstay's viewer:

```sh
JACKSTAY_VIEWER=/path/to/jackstay/build/viewer/capture-viewer-sdl \
  cargo test -p portholed --test cpu_viewer_e2e --locked -- --ignored
```

These checks use synthetic frames and the in-memory adapter. They do not prove
desktop capture, long playback or GPU completion. Separate live CPU and native
long-playback and delayed-consumer checks now pass; live resize remains pending.
See [live acceptance](../../../2026-09-12-live-acquisition-acceptance.md). Jackstay's standalone CPU producer and viewer now use the
common arena too; process-wide graceful daemon drainage remains open.

Workspace build, non-ignored tests, all-target Clippy and pinned formatting pass
on macOS and paneer Linux against the immutable Jackstay pin above. The SDL
end-to-end check passes on macOS. Linux's SDL check still needs its build
dependencies; Linux Rust tests do not establish viewer playback. Local test logs
are `/tmp/porthole-delayed-acquisition-tests.log` and, on paneer,
`/tmp/porthole-delayed-acquisition-linux-tests.log`. The delayed CPU viewer run
is recorded in `/tmp/porthole-delayed-cpu-viewer.log`.
