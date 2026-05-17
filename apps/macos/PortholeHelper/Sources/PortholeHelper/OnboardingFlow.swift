import Foundation

struct OnboardingFlow {
    enum State: Equatable {
        case idle
        case loadingInfo
        case ready(info: InfoResponse, activePermission: SystemPermissionStatus?)
        case requesting(permission: String)
        case waitingForUser(permission: String, outcome: SystemPermissionPromptOutcome)
        case restarting(permission: String)
        case verifying(permission: String)
        case complete(InfoResponse)
        case blocked(message: String)

        var activePermissionName: String? {
            switch self {
            case .ready(_, let active):
                active?.name
            case .requesting(let permission),
                 .waitingForUser(let permission, _),
                 .restarting(let permission),
                 .verifying(let permission):
                permission
            default:
                nil
            }
        }
    }

    enum Event: Equatable {
        case loadStarted
        case loadedInfo(InfoResponse)
        case loadFailed(String)
        case requestStarted(String)
        case requestSucceeded(SystemPermissionPromptOutcome)
        case requestFailed(String)
        case userConfirmedGrant(String, requiresRestart: Bool)
        case verificationSucceeded(InfoResponse)
        case verificationFailed(String)
        case restartTimedOut(String)
    }

    private(set) var state: State = .idle

    mutating func apply(_ event: Event) {
        switch event {
        case .loadStarted:
            state = .loadingInfo
        case .loadedInfo(let info):
            applyLoadedInfo(info)
        case .loadFailed(let message):
            state = .blocked(message: message)
        case .requestStarted(let permission):
            state = .requesting(permission: permission)
        case .requestSucceeded(let outcome):
            state = .waitingForUser(permission: outcome.permission, outcome: outcome)
        case .requestFailed(let message):
            state = .blocked(message: message)
        case .userConfirmedGrant(let permission, let requiresRestart):
            state = requiresRestart ? .restarting(permission: permission) : .verifying(permission: permission)
        case .verificationSucceeded(let info):
            applyLoadedInfo(info)
        case .verificationFailed(let message):
            state = .blocked(message: message)
        case .restartTimedOut(let permission):
            state = .blocked(message: "Daemon failed to restart while verifying \(permission)")
        }
    }

    private mutating func applyLoadedInfo(_ info: InfoResponse) {
        let missing = info.adapters.flatMap(\.systemPermissions).first { !$0.granted }
        if let missing {
            state = .ready(info: info, activePermission: missing)
        } else {
            state = .complete(info)
        }
    }
}
