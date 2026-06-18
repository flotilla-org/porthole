import Foundation

/// Supervises portholed as a launchd-managed LaunchAgent (dockerd model).
///
/// The helper no longer spawns portholed as a child process. Instead it
/// registers the bundled LaunchAgent via `SMAppService.agent`; launchd then
/// owns the daemon's lifecycle (RunAtLoad + KeepAlive) and — crucially — is the
/// job that vends the attach MachService. The supervisor's job is therefore to
/// (1) register the agent, (2) poll daemon liveness over the control socket to
/// drive the status UI, and (3) restart the daemon on demand for onboarding via
/// `launchctl kickstart -k`. Quitting the helper does NOT stop the daemon.
@MainActor
final class DaemonSupervisor {
    enum State: Equatable {
        /// Registering the agent / waiting for the first liveness result.
        case registering
        /// Agent registered and the daemon answers the control socket.
        case running
        /// Agent registered but the daemon is not (yet) answering.
        case unresponsive
        /// SMAppService needs the user to approve the agent in System Settings.
        case needsApproval
        /// Registration failed (e.g. invalid signature).
        case failed(String)
    }

    /// Whether the helper can trigger a daemon restart right now. Onboarding
    /// gates its restart-for-permission step on this.
    enum RestartCapability: Equatable {
        case available
        case unavailable
    }

    private let label: String
    private let cliURL: URL
    private let pollInterval: TimeInterval
    private let probeLiveness: @Sendable (URL) -> Bool
    private let registerAgent: @Sendable () -> DaemonAgentRegistrar.RegistrationResult
    private let restartService: @Sendable (String) -> Bool
    private let onStateChange: (State) -> Void

    private(set) var currentState: State = .registering
    private var pollTimer: Timer?
    private var supervising = false
    private var pollInFlight = false

    init(
        cliURL: URL,
        label: String = DaemonAgentRegistrar.label,
        pollInterval: TimeInterval = 2,
        probeLiveness: @escaping @Sendable (URL) -> Bool = DaemonSupervisor.daemonIsResponding,
        registerAgent: @escaping @Sendable () -> DaemonAgentRegistrar.RegistrationResult = { DaemonAgentRegistrar().registerIfNeeded() },
        restartService: @escaping @Sendable (String) -> Bool = DaemonSupervisor.kickstart,
        onStateChange: @escaping (State) -> Void
    ) {
        self.cliURL = cliURL
        self.label = label
        self.pollInterval = pollInterval
        self.probeLiveness = probeLiveness
        self.registerAgent = registerAgent
        self.restartService = restartService
        self.onStateChange = onStateChange
    }

    var restartCapability: RestartCapability {
        switch currentState {
        case .running, .unresponsive, .registering:
            .available
        case .needsApproval, .failed:
            .unavailable
        }
    }

    func start() {
        guard !supervising else { return }
        supervising = true
        setState(.registering)
        poll()
        startPollTimer()
    }

    /// Force a daemon restart (kill + relaunch under launchd) and re-probe.
    /// Used by onboarding to pick up a freshly granted TCC permission.
    func restart() {
        guard supervising else {
            start()
            return
        }
        let label = label
        let restartService = restartService
        DispatchQueue.global(qos: .utility).async {
            _ = restartService(label)
            Task { @MainActor [weak self] in
                self?.poll()
            }
        }
    }

    /// Stop supervising (e.g. the helper is quitting). launchd keeps the daemon
    /// alive — this only tears down the helper's polling.
    func stopForQuit() {
        supervising = false
        stopPollTimer()
    }

    private func poll() {
        guard supervising, !pollInFlight else { return }
        pollInFlight = true

        let registerAgent = registerAgent
        let probeLiveness = probeLiveness
        DispatchQueue.global(qos: .utility).async { [cliURL] in
            let registration = registerAgent()
            // Only probe the socket once the agent is actually registered;
            // there is no point probing while approval is pending.
            let alive: Bool
            switch registration {
            case .registered, .alreadyEnabled:
                alive = probeLiveness(cliURL)
            case .requiresApproval, .failed:
                alive = false
            }
            Task { @MainActor [weak self] in
                guard let self else { return }
                pollInFlight = false
                guard supervising else { return }
                apply(registration: registration, alive: alive)
            }
        }
    }

    private func apply(registration: DaemonAgentRegistrar.RegistrationResult, alive: Bool) {
        switch registration {
        case .failed(let reason):
            setState(.failed(reason))
        case .requiresApproval:
            setState(.needsApproval)
        case .registered, .alreadyEnabled:
            setState(alive ? .running : .unresponsive)
        }
    }

    private func setState(_ state: State) {
        guard state != currentState else { return }
        currentState = state
        onStateChange(state)
    }

    private func startPollTimer() {
        stopPollTimer()
        pollTimer = Timer.scheduledTimer(withTimeInterval: pollInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.poll()
            }
        }
    }

    private func stopPollTimer() {
        pollTimer?.invalidate()
        pollTimer = nil
    }

    nonisolated private static func daemonIsResponding(cliURL: URL) -> Bool {
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
            NSLog("failed to probe daemon liveness: \(error)")
            return false
        }
    }

    nonisolated private static func kickstart(label: String) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["kickstart", "-k", "gui/\(getuid())/\(label)"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice

        do {
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus == 0
        } catch {
            NSLog("failed to kickstart \(label): \(error)")
            return false
        }
    }
}
