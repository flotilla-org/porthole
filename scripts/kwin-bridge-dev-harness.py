#!/usr/bin/env python3
"""Development harness for the Porthole KWin control script.

This owns the temporary session-bus service that the KWin control script calls
during the spike branch. It prints each PublishSnapshot payload as formatted
JSON so the script can be verified before portholed owns the real bridge.
"""

import asyncio
import json
import signal

from dbus_next.aio import MessageBus
from dbus_next.service import ServiceInterface, method


SERVICE = "work.flotilla.Porthole.KWin"
PATH = "/work/flotilla/Porthole/KWin"
IFACE = "work.flotilla.Porthole.KWin"


class KWinBridgeHarness(ServiceInterface):
    def __init__(self):
        super().__init__(IFACE)

    @method()
    def PublishSnapshot(self, payload: "s") -> "b":
        try:
            parsed = json.loads(payload)
            print(json.dumps(parsed, indent=2, sort_keys=True), flush=True)
        except json.JSONDecodeError:
            print(payload, flush=True)
        return True


async def main():
    bus = await MessageBus().connect()
    bus.export(PATH, KWinBridgeHarness())
    await bus.request_name(SERVICE)
    print(f"listening on {SERVICE} {PATH}", flush=True)

    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    await stop.wait()


if __name__ == "__main__":
    asyncio.run(main())
