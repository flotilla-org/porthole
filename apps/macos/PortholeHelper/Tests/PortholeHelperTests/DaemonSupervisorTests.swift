import XCTest
@testable import PortholeHelper

@MainActor
final class DaemonSupervisorTests: XCTestCase {
    func testInitialRestartCapabilityIsHelperOwned() {
        let supervisor = DaemonSupervisor(
            daemonURL: URL(fileURLWithPath: "/tmp/portholed"),
            cliURL: URL(fileURLWithPath: "/tmp/porthole")
        ) { _ in }

        XCTAssertEqual(supervisor.currentState, .stopped)
        XCTAssertEqual(supervisor.restartCapability, .helperOwned)
    }
}
