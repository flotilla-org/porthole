import Foundation

@MainActor
final class DaemonSupervisor {
    enum State: Equatable {
        case stopped
        case running(pid: Int32)
        case crashed(status: Int32)
    }

    private let daemonURL: URL
    private var process: Process?
    private var shouldRestart = true
    private let onStateChange: (State) -> Void

    init(daemonURL: URL, onStateChange: @escaping (State) -> Void) {
        self.daemonURL = daemonURL
        self.onStateChange = onStateChange
    }

    func start() {
        guard process == nil else { return }
        shouldRestart = true

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
}
