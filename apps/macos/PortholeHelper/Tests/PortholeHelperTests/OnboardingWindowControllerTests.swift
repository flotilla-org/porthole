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

    func testBlockedStateHasNoExtraRefreshButton() async {
        let supervisor = DaemonSupervisor(
            daemonURL: URL(fileURLWithPath: "/tmp/portholed"),
            cliURL: URL(fileURLWithPath: "/tmp/porthole")
        ) { _ in }
        let controller = OnboardingWindowController(client: FailingPortholeClient(), supervisor: supervisor)

        let status = try? await waitForLabel("Onboarding status", in: controller) { $0 == "Action needed" }
        XCTAssertEqual(status, "Action needed")

        let refreshButtons = visibleButtons(in: controller).filter { $0.title == "Refresh" }
        XCTAssertEqual(refreshButtons.count, 1)
    }

    func testRendersPermissionRowsAfterInfoLoad() async throws {
        let supervisor = DaemonSupervisor(
            daemonURL: URL(fileURLWithPath: "/tmp/portholed"),
            cliURL: URL(fileURLWithPath: "/tmp/porthole")
        ) { _ in }
        let controller = OnboardingWindowController(
            client: FakePortholeClient(info: OnboardingFixtures.info([
                .init(name: "accessibility", granted: false, purpose: "input"),
                .init(name: "screen_recording", granted: false, purpose: "capture"),
            ])),
            supervisor: supervisor
        )

        let status = try await waitForLabel("Onboarding status", in: controller) { $0 == "Accessibility permission needed" }
        XCTAssertEqual(status, "Accessibility permission needed")
        XCTAssertTrue(labels(in: controller.window?.contentView).contains { $0.stringValue == "Accessibility" })
        XCTAssertTrue(labels(in: controller.window?.contentView).contains { $0.stringValue == "Screen Recording" })
    }

    private func waitForLabel(
        _ accessibilityLabel: String,
        in controller: OnboardingWindowController,
        matching predicate: @escaping (String) -> Bool
    ) async throws -> String {
        let deadline = Date().addingTimeInterval(1)
        var lastObservedValue: String?
        while Date() < deadline {
            let matchingLabels = labels(in: controller.window?.contentView).filter {
                $0.accessibilityLabel() == accessibilityLabel
            }
            lastObservedValue = matchingLabels.last?.stringValue ?? lastObservedValue
            if let label = matchingLabels.first(where: { predicate($0.stringValue) }) {
                return label.stringValue
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let actualValue = lastObservedValue.map { "'\($0)'" } ?? "not found"
        throw NSError(
            domain: "OnboardingWindowControllerTests",
            code: 1,
            userInfo: [
                NSLocalizedDescriptionKey: "Timed out waiting for label '\(accessibilityLabel)' to match expected value; actual value was \(actualValue)"
            ]
        )
    }

    private func visibleButtons(in controller: OnboardingWindowController) -> [NSButton] {
        buttons(in: controller.window?.contentView).filter { !$0.isHidden }
    }

    private func buttons(in root: NSView?) -> [NSButton] {
        guard let root else { return [] }
        return root.subviews.flatMap { view -> [NSButton] in
            let current = (view as? NSButton).map { [$0] } ?? []
            return current + buttons(in: view)
        }
    }

    private func labels(in root: NSView?) -> [NSTextField] {
        guard let root else { return [] }
        return root.subviews.flatMap { view -> [NSTextField] in
            let current = (view as? NSTextField).map { [$0] } ?? []
            return current + labels(in: view)
        }
    }
}

private struct FakePortholeClient: PortholeClientProtocol {
    var info: InfoResponse = InfoResponse(daemonVersion: "test", uptimeSeconds: 1, adapters: [])

    func info() async throws -> InfoResponse {
        info
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

private struct FailingPortholeClient: PortholeClientProtocol {
    func info() async throws -> InfoResponse {
        throw PortholeClientError.connectionFailed("daemon unavailable")
    }

    func requestPermissionPrompt(name: String) async throws -> SystemPermissionPromptOutcome {
        throw PortholeClientError.connectionFailed("daemon unavailable")
    }
}
