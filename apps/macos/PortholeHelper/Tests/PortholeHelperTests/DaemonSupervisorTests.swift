import XCTest
@testable import PortholeHelper

@MainActor
final class DaemonSupervisorTests: XCTestCase {
    private final class ProbeScript: @unchecked Sendable {
        private let lock = NSLock()
        private var results: [Bool]

        init(_ results: [Bool]) {
            self.results = results
        }

        func next(_ url: URL) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard !results.isEmpty else { return true }
            return results.removeFirst()
        }
    }

    private final class LaunchRecorder: @unchecked Sendable {
        private(set) var count = 0

        func launch(_ _: URL, onTermination: @escaping @Sendable (Int32) -> Void) -> DaemonSupervisor.ManagedDaemon {
            count += 1
            return DaemonSupervisor.ManagedDaemon(pid: 42, terminate: {})
        }
    }

    func testInitialRestartCapabilityIsHelperOwned() {
        let supervisor = DaemonSupervisor(
            daemonURL: URL(fileURLWithPath: "/tmp/portholed"),
            cliURL: URL(fileURLWithPath: "/tmp/porthole")
        ) { _ in }

        XCTAssertEqual(supervisor.currentState, .stopped)
        XCTAssertEqual(supervisor.restartCapability, .helperOwned)
    }

    func testRunningExternalReprobeLaunchesBundledDaemonWhenExternalExits() async throws {
        let probeResults = ProbeScript([true, false])
        let launcher = LaunchRecorder()
        var states: [DaemonSupervisor.State] = []
        let recovered = expectation(description: "helper launched bundled daemon after external daemon exited")

        let supervisor = DaemonSupervisor(
            daemonURL: URL(fileURLWithPath: "/tmp/portholed"),
            cliURL: URL(fileURLWithPath: "/tmp/porthole"),
            externalReprobeInterval: 0.01,
            probeExistingDaemon: probeResults.next,
            launchDaemonProcess: launcher.launch
        ) { state in
            states.append(state)
            if state == .running(pid: 42) {
                recovered.fulfill()
            }
        }

        supervisor.start()
        await fulfillment(of: [recovered], timeout: 1.0)

        XCTAssertEqual(launcher.count, 1)
        XCTAssertEqual(supervisor.currentState, .running(pid: 42))
        XCTAssertEqual(states, [.runningExternal, .running(pid: 42)])
        XCTAssertEqual(supervisor.restartCapability, .helperOwned)
    }
}
