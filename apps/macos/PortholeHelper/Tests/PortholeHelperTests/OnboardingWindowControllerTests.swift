import AppKit
import XCTest
@testable import PortholeHelper

@MainActor
final class OnboardingWindowControllerTests: XCTestCase {
    func testCreatesOnboardingWindow() {
        let supervisor = DaemonSupervisor(
            daemonURL: URL(fileURLWithPath: "/tmp/portholed"),
            cliURL: URL(fileURLWithPath: "/tmp/porthole")
        ) { _ in }
        let controller = OnboardingWindowController(client: FakePortholeClient(), supervisor: supervisor)

        XCTAssertEqual(controller.window?.title, "Porthole Onboarding")
        XCTAssertEqual(controller.window?.contentMinSize, NSSize(width: 480, height: 360))
    }
}

private struct FakePortholeClient: PortholeClientProtocol {
    func info() async throws -> InfoResponse {
        InfoResponse(daemonVersion: "test", uptimeSeconds: 1, adapters: [])
    }

    func requestPermissionPrompt(name: String) async throws -> SystemPermissionPromptOutcome {
        SystemPermissionPromptOutcome(
            permission: name,
            grantedBefore: false,
            grantedAfter: false,
            requiresDaemonRestart: false,
            notes: "test"
        )
    }
}
