/*
 * Porthole KWin control script.
 *
 * This script is intentionally tiny at first: it proves that a KWin-hosted
 * script can observe compositor state and call out to Porthole's session-bus
 * bridge. The real command protocol lands in the kwin-dbus-bridge branch.
 */

const SERVICE = "work.flotilla.Porthole.KWin";
const PATH = "/work/flotilla/Porthole/KWin";
const IFACE = "work.flotilla.Porthole.KWin";
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

function windowToJson(window) {
    return {
        caption: readString(window.caption),
        resourceClass: readString(window.resourceClass),
        resourceName: readString(window.resourceName),
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
    return {
        schemaVersion: 1,
        reason,
        activeWindow: active,
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
