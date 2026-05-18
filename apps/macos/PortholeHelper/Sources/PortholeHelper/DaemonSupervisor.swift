import Foundation

@MainActor
final class DaemonSupervisor {
    struct ManagedDaemon {
        let pid: Int32
        let terminate: () -> Void
    }

    enum State: Equatable {
        case stopped
        case running(pid: Int32)
        case runningExternal
        case crashed(status: Int32)
    }

    enum RestartCapability: Equatable {
        case helperOwned
        case external
    }

    private let daemonURL: URL
    private let cliURL: URL
    private let externalReprobeInterval: TimeInterval
    private let probeExistingDaemon: @Sendable (URL) -> Bool
    private let launchDaemonProcess: @Sendable (URL, @escaping @Sendable (Int32) -> Void) throws -> ManagedDaemon
    private(set) var currentState: State = .stopped
    private var process: ManagedDaemon?
    private var externalReprobeTimer: Timer?
    private var shouldRestart = true
    private var startupProbeInFlight = false
    private let onStateChange: (State) -> Void

    init(
        daemonURL: URL,
        cliURL: URL,
        externalReprobeInterval: TimeInterval = 5,
        probeExistingDaemon: @escaping @Sendable (URL) -> Bool = DaemonSupervisor.daemonIsAlreadyRunning,
        launchDaemonProcess: @escaping @Sendable (URL, @escaping @Sendable (Int32) -> Void) throws -> ManagedDaemon = DaemonSupervisor.launchProcess,
        onStateChange: @escaping (State) -> Void
    ) {
        self.daemonURL = daemonURL
        self.cliURL = cliURL
        self.externalReprobeInterval = externalReprobeInterval
        self.probeExistingDaemon = probeExistingDaemon
        self.launchDaemonProcess = launchDaemonProcess
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

        let probeExistingDaemon = probeExistingDaemon
        DispatchQueue.global(qos: .utility).async { [cliURL] in
            let alreadyRunning = probeExistingDaemon(cliURL)
            Task { @MainActor [weak self] in
                guard let self else { return }
                startupProbeInFlight = false
                guard shouldRestart else { return }
                guard process == nil else { return }
                if alreadyRunning {
                    setState(.runningExternal)
                    startExternalReprobeTimer()
                    return
                }
                launchDaemon()
            }
        }
    }

    private func launchDaemon() {
        do {
            stopExternalReprobeTimer()
            let next = try launchDaemonProcess(daemonURL) { [weak self] status in
                Task { @MainActor in
                    self?.handleTermination(status)
                }
            }
            process = next
            setState(.running(pid: next.pid))
        } catch {
            NSLog("failed to launch portholed: \(error)")
            setState(.crashed(status: -1))
        }
    }

    func restart() {
        shouldRestart = true
        stopExternalReprobeTimer()
        if let process {
            process.terminate()
        } else {
            start()
        }
    }

    func stopForQuit() {
        shouldRestart = false
        startupProbeInFlight = false
        stopExternalReprobeTimer()
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

    private func startExternalReprobeTimer() {
        stopExternalReprobeTimer()
        externalReprobeTimer = Timer.scheduledTimer(withTimeInterval: externalReprobeInterval, repeats: true) { [weak self] _ in
            guard let self else { return }
            // Timer callbacks enter from the run loop; hop into the actor
            // before touching supervisor state.
            Task { @MainActor in
                self.reprobeExternalDaemon()
            }
        }
    }

    private func stopExternalReprobeTimer() {
        externalReprobeTimer?.invalidate()
        externalReprobeTimer = nil
    }

    private func reprobeExternalDaemon() {
        guard currentState == .runningExternal else {
            stopExternalReprobeTimer()
            return
        }
        let probeExistingDaemon = probeExistingDaemon
        DispatchQueue.global(qos: .utility).async { [cliURL] in
            let stillRunning = probeExistingDaemon(cliURL)
            Task { @MainActor [weak self] in
                guard let self else { return }
                guard shouldRestart, currentState == .runningExternal else { return }
                if !stillRunning {
                    launchDaemon()
                }
            }
        }
    }

    nonisolated private static func launchProcess(daemonURL: URL, onTermination: @escaping @Sendable (Int32) -> Void) throws -> ManagedDaemon {
        let process = Process()
        process.executableURL = daemonURL
        process.terminationHandler = { terminated in
            onTermination(terminated.terminationStatus)
        }
        try process.run()
        return ManagedDaemon(pid: process.processIdentifier) {
            process.terminate()
        }
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
