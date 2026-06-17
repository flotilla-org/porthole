import XCTest
@testable import PortholeHelper

@MainActor
final class DaemonSupervisorTests: XCTestCase {
    private final class LabelRecorder: @unchecked Sendable {
        private let lock = NSLock()
        private var labels: [String] = []

        func record(_ label: String) {
            lock.lock()
            defer { lock.unlock() }
            labels.append(label)
        }

        func snapshot() -> [String] {
            lock.lock()
            defer { lock.unlock() }
            return labels
        }
    }

    // A large poll interval keeps each test to the single immediate poll that
    // start()/restart() trigger, so state transitions are deterministic.
    private func makeSupervisor(
        registration: DaemonAgentRegistrar.RegistrationResult,
        alive: Bool,
        restartService: @escaping @Sendable (String) -> Bool = { _ in true },
        onStateChange: @escaping (DaemonSupervisor.State) -> Void
    ) -> DaemonSupervisor {
        DaemonSupervisor(
            cliURL: URL(fileURLWithPath: "/tmp/porthole"),
            pollInterval: 100,
            probeLiveness: { _ in alive },
            registerAgent: { registration },
            restartService: restartService,
            onStateChange: onStateChange
        )
    }

    func testInitialStateIsRegisteringAndRestartable() {
        let supervisor = DaemonSupervisor(cliURL: URL(fileURLWithPath: "/tmp/porthole")) { _ in }

        XCTAssertEqual(supervisor.currentState, .registering)
        XCTAssertEqual(supervisor.restartCapability, .available)
    }

    func testRegisteredAndAliveBecomesRunning() async {
        let running = expectation(description: "running")
        let supervisor = makeSupervisor(registration: .alreadyEnabled, alive: true) { state in
            if state == .running { running.fulfill() }
        }

        supervisor.start()
        await fulfillment(of: [running], timeout: 1.0)

        XCTAssertEqual(supervisor.currentState, .running)
        XCTAssertEqual(supervisor.restartCapability, .available)
    }

    func testRegisteredButNotAliveBecomesUnresponsive() async {
        let unresponsive = expectation(description: "unresponsive")
        let supervisor = makeSupervisor(registration: .registered, alive: false) { state in
            if state == .unresponsive { unresponsive.fulfill() }
        }

        supervisor.start()
        await fulfillment(of: [unresponsive], timeout: 1.0)

        XCTAssertEqual(supervisor.currentState, .unresponsive)
        XCTAssertEqual(supervisor.restartCapability, .available)
    }

    func testRequiresApprovalBecomesNeedsApprovalAndIsNotRestartable() async {
        let needsApproval = expectation(description: "needsApproval")
        let supervisor = makeSupervisor(registration: .requiresApproval, alive: false) { state in
            if state == .needsApproval { needsApproval.fulfill() }
        }

        supervisor.start()
        await fulfillment(of: [needsApproval], timeout: 1.0)

        XCTAssertEqual(supervisor.currentState, .needsApproval)
        XCTAssertEqual(supervisor.restartCapability, .unavailable)
    }

    func testFailedRegistrationBecomesFailedAndIsNotRestartable() async {
        let failed = expectation(description: "failed")
        let supervisor = makeSupervisor(registration: .failed("invalid signature"), alive: false) { state in
            if state == .failed("invalid signature") { failed.fulfill() }
        }

        supervisor.start()
        await fulfillment(of: [failed], timeout: 1.0)

        XCTAssertEqual(supervisor.currentState, .failed("invalid signature"))
        XCTAssertEqual(supervisor.restartCapability, .unavailable)
    }

    func testRestartKickstartsConfiguredLabel() async throws {
        let recorder = LabelRecorder()
        let supervisor = DaemonSupervisor(
            cliURL: URL(fileURLWithPath: "/tmp/porthole"),
            pollInterval: 100,
            probeLiveness: { _ in true },
            registerAgent: { .alreadyEnabled },
            restartService: { label in
                recorder.record(label)
                return true
            }
        ) { _ in }

        supervisor.start()
        supervisor.restart()

        let deadline = Date().addingTimeInterval(1)
        while recorder.snapshot().isEmpty, Date() < deadline {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertEqual(recorder.snapshot(), [DaemonAgentRegistrar.label])
    }
}
