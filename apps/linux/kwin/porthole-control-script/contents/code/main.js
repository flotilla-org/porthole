/*
 * Porthole KWin control script.
 *
 * This script is intentionally tiny at first: it proves that a KWin-hosted
 * script can observe compositor state and call out to Porthole's session-bus
 * bridge.
 */

const SERVICE = "work.flotilla.Porthole.KWin";
const PATH = "/work/flotilla/Porthole/KWin";
const IFACE = "work.flotilla.Porthole.KWin";
const SCRIPT_INSTANCE_ID = "porthole-control";
const connectedWindows = new Map();

function log(message) {
    console.info("porthole-control: " + message);
}

function readString(value) {
    if (value === undefined || value === null) {
        return null;
    }
    return String(value);
}

function readBool(value) {
    return value === true;
}

function rectToJson(rect) {
    if (!rect) {
        return null;
    }
    return {
        x: Number(rect.x),
        y: Number(rect.y),
        width: Number(rect.width),
        height: Number(rect.height),
    };
}

function outputName(output) {
    if (!output) {
        return null;
    }
    return readString(output.name || output.model || output);
}

function outputToJson(output) {
    return {
        name: outputName(output),
        geometry: rectToJson(output ? output.geometry : null),
        scale: Number((output && output.devicePixelRatio) || 1),
        active: workspace.activeScreen === output,
    };
}

function cursorToJson() {
    const pos = workspace.cursorPos || { x: 0, y: 0 };
    const output = workspace.screenAt ? workspace.screenAt(pos) : null;
    return {
        x: Number(pos.x || 0),
        y: Number(pos.y || 0),
        output: outputName(output),
    };
}

function windowToJson(window) {
    return {
        windowId: readString(window.internalId || window.windowId) || "",
        caption: readString(window.caption),
        resourceClass: readString(window.resourceClass),
        resourceName: readString(window.resourceName),
        desktopFileName: readString(window.desktopFileName),
        pid: Number(window.pid || 0),
        normalWindow: readBool(window.normalWindow),
        active: readBool(window.active),
        minimized: readBool(window.minimized),
        output: outputName(window.output),
        frameGeometry: rectToJson(window.frameGeometry),
    };
}

function buildSnapshot(reason) {
    const windows = workspace.windowList().map(windowToJson);
    const active = workspace.activeWindow ? windowToJson(workspace.activeWindow) : null;
    const outputs = workspace.screens ? workspace.screens.map(outputToJson) : [];
    return {
        schemaVersion: 1,
        reason,
        activeWindow: active,
        cursor: cursorToJson(),
        outputs,
        windowCount: windows.length,
        windows,
    };
}

function publishSnapshot(reason) {
    const payload = JSON.stringify(buildSnapshot(reason));
    callDBus(SERVICE, PATH, IFACE, "PublishSnapshot", payload, function () {
        // The bridge is optional during development; failed calls are visible
        // in KWin logs. Keep the script running either way.
    });
}

function completeCommand(commandId, result) {
    callDBus(SERVICE, PATH, IFACE, "CompleteCommand", String(commandId), JSON.stringify(result), function () {
        // Command completions are best-effort from the script side. The daemon
        // will retry or surface a timeout in later adapter branches.
    });
}

function findWindow(windowId) {
    const wanted = String(windowId || "");
    const matches = workspace.windowList().filter(function (window) {
        return readString(window.internalId || window.windowId) === wanted;
    });
    return matches.length > 0 ? matches[0] : null;
}

function handleCommand(commandJson) {
    if (!commandJson) {
        return;
    }
    let command = null;
    try {
        command = JSON.parse(String(commandJson));
    } catch (error) {
        log("failed to parse command: " + error);
        return;
    }
    const commandId = command.commandId || "";
    const windowId = command.payload ? command.payload.windowId : "";
    const args = command.payload && command.payload.args ? command.payload.args : {};
    const window = findWindow(windowId);
    if (!window) {
        completeCommand(commandId, { ok: false, error: "window_not_found" });
        return;
    }
    try {
        if (command.kind === "focus") {
            workspace.activeWindow = window;
        } else if (command.kind === "close") {
            window.closeWindow();
        } else if (command.kind === "place_surface") {
            window.frameGeometry = {
                x: Number(args.x),
                y: Number(args.y),
                width: Number(args.width),
                height: Number(args.height),
            };
        } else {
            completeCommand(commandId, {
                ok: false,
                error: "unsupported_command",
                kind: command.kind || null,
            });
            return;
        }
        publishSnapshot("command-" + String(command.kind || "unknown"));
        completeCommand(commandId, { ok: true });
    } catch (error) {
        completeCommand(commandId, { ok: false, error: String(error) });
    }
}

function pollCommands() {
    callDBus(SERVICE, PATH, IFACE, "NextCommand", SCRIPT_INSTANCE_ID, function (commandJson) {
        handleCommand(commandJson);
        pollCommands();
    });
}

function connectWindow(window) {
    if (!window || connectedWindows.has(window)) {
        return;
    }
    connectedWindows.set(window, true);
    if (window.frameGeometryChanged) {
        window.frameGeometryChanged.connect(function () {
            publishSnapshot("window-geometry-changed");
        });
    }
    if (window.closed) {
        window.closed.connect(function () {
            connectedWindows.delete(window);
            publishSnapshot("window-closed");
        });
    }
}

function connectExistingWindows() {
    workspace.windowList().forEach(connectWindow);
}

function main() {
    log("starting");
    connectExistingWindows();
    publishSnapshot("startup");
    pollCommands();

    workspace.windowAdded.connect(function (window) {
        connectWindow(window);
        publishSnapshot("window-added");
    });

    if (workspace.windowActivated) {
        workspace.windowActivated.connect(function () {
            publishSnapshot("window-activated");
        });
    }

    if (workspace.currentDesktopChanged) {
        workspace.currentDesktopChanged.connect(function () {
            publishSnapshot("desktop-changed");
        });
    }
}

main();
