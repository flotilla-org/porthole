import Foundation
import ServiceManagement

struct LoginItemRegistrar {
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

        static func mainApp() -> Self {
            let service = SMAppService.mainApp
            return Self(
                status: { ServiceStatus(service.status) },
                register: { try service.register() }
            )
        }
    }

    private let dependencies: Dependencies

    init(dependencies: Dependencies = .mainApp()) {
        self.dependencies = dependencies
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

private extension LoginItemRegistrar.ServiceStatus {
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
