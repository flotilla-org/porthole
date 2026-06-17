import Foundation
import ServiceManagement

/// Registers portholed as its own launchd-managed LaunchAgent via
/// `SMAppService.agent`, using the bundled plist under
/// `Contents/Library/LaunchAgents/`. This keeps the daemon a separate launchd
/// job (dockerd model) so it — not the helper — vends the attach MachService
/// and owns its TCC grants. Mirrors `LoginItemRegistrar`.
struct DaemonAgentRegistrar {
    /// Bundled plist file name; matches the file copied by the xtask bundler
    /// and the `Label` declared inside it.
    static let plistName = "work.flotilla.porthole.daemon.plist"
    static let label = "work.flotilla.porthole.daemon"

    enum ServiceStatus: Equatable {
        case notRegistered
        case enabled
        case requiresApproval
        case notFound
        case unknown
    }

    enum RegistrationResult: Equatable {
        case alreadyEnabled
        case registered
        case requiresApproval
        case failed(String)
    }

    struct Dependencies {
        var status: () -> ServiceStatus
        var register: () throws -> Void

        static func agent(plistName: String = DaemonAgentRegistrar.plistName) -> Self {
            let service = SMAppService.agent(plistName: plistName)
            return Self(
                status: { ServiceStatus(service.status) },
                register: { try service.register() }
            )
        }
    }

    private let dependencies: Dependencies

    init(dependencies: Dependencies = .agent()) {
        self.dependencies = dependencies
    }

    func currentStatus() -> ServiceStatus {
        dependencies.status()
    }

    func registerIfNeeded() -> RegistrationResult {
        switch dependencies.status() {
        case .enabled:
            return .alreadyEnabled
        case .requiresApproval:
            return .requiresApproval
        case .notRegistered, .notFound:
            break
        case .unknown:
            return .failed("unknown SMAppService status")
        }

        do {
            try dependencies.register()
            let nextStatus = dependencies.status()
            if nextStatus == .requiresApproval {
                return .requiresApproval
            }
            if nextStatus == .unknown {
                return .failed("unknown SMAppService status")
            }
            return .registered
        } catch {
            if dependencies.status() == .requiresApproval {
                return .requiresApproval
            }
            return .failed(String(describing: error))
        }
    }
}

private extension DaemonAgentRegistrar.ServiceStatus {
    init(_ status: SMAppService.Status) {
        switch status {
        case .notRegistered:
            self = .notRegistered
        case .enabled:
            self = .enabled
        case .requiresApproval:
            self = .requiresApproval
        case .notFound:
            self = .notFound
        @unknown default:
            self = .unknown
        }
    }
}
