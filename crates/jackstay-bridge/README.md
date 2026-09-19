# jackstay-bridge

The cross-host bridge: an egress half that encodes a local Jackstay
publication and sends it over a link, and an ingress half that decodes what
arrives and republishes it. This crate lives in Porthole and depends on the
Jackstay transport and graph libraries. See [the design note](../../docs/jackstay-bridge.md).

Moved from `flotilla-org/jackstay` at `2be1314fd33c60bb4cf137dafe57f1a7fa823bc1`;
the executable name and wire protocol are unchanged.

Porthole runs `task::Task` instances inside its daemon by default. A task owns
its threads and sockets; stopping or dropping it interrupts blocked link I/O
and joins its threads. The separate executable remains available for
`porthole publications export/republish/attach --worker` and standalone probes.

```sh
cargo build -p jackstay-bridge
target/debug/jackstay-bridge probe                 # capability probe as JSON
target/debug/jackstay-bridge loopback --frames 120 # both halves in one process, self-verifying
target/debug/jackstay-bridge loopback --viewer-service work.flotilla.jackstay.bridge.demo --viewer-token demo
build/viewer/capture-viewer-sdl --native --mach-service work.flotilla.jackstay.bridge.demo --token demo
```

The halves as processes, for a coordinator that forwarded two sockets:

```sh
jackstay-bridge egress  --media M --control C --service SOURCE --source-token T --link-token L
jackstay-bridge ingress --media M --control C --service NAME   --viewer-token V --link-token L
```

`ingress --cpu-socket PATH` (also accepted by `loopback`) additionally serves the
republication over a generic CPU setup socket at `PATH`, owner-only and unlinked
on exit, for consumers without a native attach path: the SDL viewer's
`--cpu-socket PATH`, katzensteg's `jackstay-source PATH`. Each decoded frame is
then also read back from the staging surface into a CPU arena.

`--input-socket PATH` relays the jackstay input protocol back to the producer
host. On the ingress (and `loopback`) it is where controllers connect, owner-only
like the CPU socket; on the egress it is the executor's socket that each relayed
stream is connected to. `loopback --input-socket PATH` runs a reference executor
that prints every event, so the SDL viewer with `--cpu-socket` and
`--input-socket` exercises the relay on one machine.

The macOS backend is enabled by default and included in Porthole's app bundle.
Use `--no-default-features` for just the portable wire, clock and policy code.
Native bridge execution is currently supported only on macOS.
