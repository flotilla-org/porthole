import Foundation

@MainActor
final class DaemonSupervisor {
    enum State: Equatable {
        case stopped
        case running(pid: Int32)
        case runningExternal
        case crashed(status: Int32)
    }

    private let daemonURL: URL
    private let cliURL: URL
    private var process: Process?
    private var shouldRestart = true
    private let onStateChange: (State) -> Void

    init(daemonURL: URL, cliURL: URL, onStateChange: @escaping (State) -> Void) {
        self.daemonURL = daemonURL
        self.cliURL = cliURL
        self.onStateChange = onStateChange
    }

    func start() {
        guard process == nil else { return }
        shouldRestart = true
        if daemonIsAlreadyRunning() {
            onStateChange(.runningExternal)
            return
        }

        let next = Process()
        next.executableURL = daemonURL
        next.terminationHandler = { [weak self] terminated in
            Task { @MainActor in
                self?.handleTermination(terminated.terminationStatus)
            }
        }

        do {
            try next.run()
            process = next
            onStateChange(.running(pid: next.processIdentifier))
        } catch {
            NSLog("failed to launch portholed: \(error)")
            onStateChange(.crashed(status: -1))
        }
    }

    func restart() {
        shouldRestart = true
        if let process {
            process.terminate()
        } else {
            start()
        }
    }

    func stopForQuit() {
        shouldRestart = false
        if let process {
            process.terminate()
        } else {
            onStateChange(.stopped)
        }
    }

    private func handleTermination(_ status: Int32) {
        process = nil
        if shouldRestart {
            onStateChange(.crashed(status: status))
            start()
        } else {
            onStateChange(.stopped)
        }
    }

    private func daemonIsAlreadyRunning() -> Bool {
        let probe = Process()
        probe.executableURL = cliURL
        probe.arguments = ["info"]
        probe.standardOutput = FileHandle.nullDevice
        probe.standardError = FileHandle.nullDevice

        do {
            try probe.run()
            probe.waitUntilExit()
            return probe.terminationStatus == 0
        } catch {
            NSLog("failed to probe existing daemon: \(error)")
            return false
        }
    }
}
