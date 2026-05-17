import XCTest
@testable import PortholeHelper

final class OnboardingFlowTests: XCTestCase {
    func testAllGrantedCompletesOnLoad() {
        let info = OnboardingFixtures.info([
            .init(name: "accessibility", granted: true, purpose: "input"),
        ])
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(info))
        XCTAssertEqual(flow.state, .complete(info))
    }

    func testFirstMissingPermissionBecomesActive() {
        let info = OnboardingFixtures.info([
            .init(name: "accessibility", granted: false, purpose: "input"),
            .init(name: "screen_recording", granted: false, purpose: "capture"),
        ])
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(info))
        XCTAssertEqual(flow.state.activePermissionName, "accessibility")
    }

    func testRequestOutcomeWaitsForUser() {
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(OnboardingFixtures.info([
            .init(name: "accessibility", granted: false, purpose: "input"),
        ])))
        flow.apply(.requestStarted("accessibility"))
        flow.apply(.requestSucceeded(OnboardingFixtures.outcome("accessibility", requiresRestart: true)))
        XCTAssertEqual(flow.state, .waitingForUser(permission: "accessibility", outcome: OnboardingFixtures.outcome("accessibility", requiresRestart: true)))
    }

    func testLoadFailureBlocksWithMessage() {
        var flow = OnboardingFlow()
        flow.apply(.loadStarted)
        flow.apply(.loadFailed("daemon unavailable"))
        XCTAssertEqual(flow.state, .blocked(message: "daemon unavailable"))
    }

    func testVerificationSuccessAdvancesToNextPermission() {
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(OnboardingFixtures.info([
            .init(name: "accessibility", granted: false, purpose: "input"),
        ])))
        flow.apply(.userConfirmedGrant("accessibility", requiresRestart: false))
        flow.apply(.verificationSucceeded(OnboardingFixtures.info([
            .init(name: "accessibility", granted: true, purpose: "input"),
            .init(name: "screen_recording", granted: false, purpose: "capture"),
        ])))
        XCTAssertEqual(flow.state.activePermissionName, "screen_recording")
    }

    func testVerificationFailureBlocksWithMessage() {
        var flow = OnboardingFlow()
        flow.apply(.verificationFailed("permission still missing"))
        XCTAssertEqual(flow.state, .blocked(message: "permission still missing"))
    }

    func testRestartTimeoutBlocksWithManualControls() {
        var flow = OnboardingFlow()
        flow.apply(.restartTimedOut("accessibility"))
        XCTAssertEqual(flow.state, .blocked(message: "Daemon failed to restart while verifying accessibility"))
    }
}

enum OnboardingFixtures {
    static func info(_ permissions: [SystemPermissionStatus]) -> InfoResponse {
        InfoResponse(
            daemonVersion: "test",
            uptimeSeconds: 1,
            adapters: [AdapterInfo(name: "macos", loaded: true, capabilities: ["system_permission_prompt"], systemPermissions: permissions)]
        )
    }

    static func outcome(_ permission: String, requiresRestart: Bool) -> SystemPermissionPromptOutcome {
        SystemPermissionPromptOutcome(
            permission: permission,
            grantedBefore: false,
            grantedAfter: false,
            requiresDaemonRestart: requiresRestart,
            notes: "notes"
        )
    }
}
