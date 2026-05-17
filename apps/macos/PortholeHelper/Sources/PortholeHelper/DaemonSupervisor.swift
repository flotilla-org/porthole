import Foundation

@MainActor
final class DaemonSupervisor {
    enum State: Equatable {
        case stopped
        case running(pid: Int32)
        // External daemons are detected before launching a child process. They
        // are not watched yet; the roadmap tracks passive recovery for this
        // transitional state.
        case runningExternal
        case crashed(status: Int32)
    }

    enum RestartCapability: Equatable {
        case helperOwned
        case external
    }

    private let daemonURL: URL
    private let cliURL: URL
    private(set) var currentState: State = .stopped
    private var process: Process?
    private var shouldRestart = true
    private var startupProbeInFlight = false
    private let onStateChange: (State) -> Void

    init(daemonURL: URL, cliURL: URL, onStateChange: @escaping (State) -> Void) {
        self.daemonURL = daemonURL
        self.cliURL = cliURL
        self.onStateChange = onStateChange
    }

    var restartCapability: RestartCapability {
        switch currentState {
        case .running:
            .helperOwned
        case .runningExternal:
            .external
        case .stopped, .crashed:
            .helperOwned
        }
    }

    func start() {
        guard process == nil else { return }
        guard !startupProbeInFlight else { return }
        shouldRestart = true
        startupProbeInFlight = true

        DispatchQueue.global(qos: .utility).async { [cliURL] in
            let alreadyRunning = Self.daemonIsAlreadyRunning(cliURL: cliURL)
            Task { @MainActor [weak self] in
                guard let self else { return }
                startupProbeInFlight = false
                guard shouldRestart else { return }
                guard process == nil else { return }
                if alreadyRunning {
                    setState(.runningExternal)
                    return
                }
                launchDaemon()
            }
        }
    }

    private func launchDaemon() {
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
            setState(.running(pid: next.processIdentifier))
        } catch {
            NSLog("failed to launch portholed: \(error)")
            setState(.crashed(status: -1))
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
        startupProbeInFlight = false
        if let process {
            process.terminate()
        } else {
            setState(.stopped)
        }
    }

    private func handleTermination(_ status: Int32) {
        process = nil
        if shouldRestart {
            setState(.crashed(status: status))
            start()
        } else {
            setState(.stopped)
        }
    }

    private func setState(_ state: State) {
        currentState = state
        onStateChange(state)
    }

    nonisolated private static func daemonIsAlreadyRunning(cliURL: URL) -> Bool {
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
