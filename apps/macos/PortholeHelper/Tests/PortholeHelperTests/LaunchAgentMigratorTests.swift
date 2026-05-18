import XCTest
@testable import PortholeHelper

final class LaunchAgentMigratorTests: XCTestCase {
    private final class CallRecorder: @unchecked Sendable {
        private let lock = NSLock()
        private var events: [String] = []

        func append(_ event: String) {
            lock.lock()
            defer { lock.unlock() }
            events.append(event)
        }

        func snapshot() -> [String] {
            lock.lock()
            defer { lock.unlock() }
            return events
        }
    }

    private func plistURL(home: URL) -> URL {
        home
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("LaunchAgents", isDirectory: true)
            .appendingPathComponent("org.flotilla.porthole.plist")
    }

    func testMissingPlistDoesNotRunBootout() {
        let home = URL(fileURLWithPath: "/Users/tester", isDirectory: true)
        let calls = CallRecorder()
        let migrator = LaunchAgentMigrator(
            dependencies: .init(
                homeDirectory: { home },
                fileExists: { _ in false },
                bootout: { _ in calls.append("bootout") },
                removeFile: { _ in calls.append("remove") }
            )
        )

        XCTAssertEqual(migrator.migrate(), .notNeeded)
        XCTAssertEqual(calls.snapshot(), [])
    }

    func testExistingPlistBootsOutThenRemovesFile() {
        let home = URL(fileURLWithPath: "/Users/tester", isDirectory: true)
        let expectedPlistURL = plistURL(home: home)
        let calls = CallRecorder()
        let migrator = LaunchAgentMigrator(
            dependencies: .init(
                homeDirectory: { home },
                fileExists: { url in
                    XCTAssertEqual(url, expectedPlistURL)
                    return true
                },
                bootout: { url in
                    XCTAssertEqual(url, expectedPlistURL)
                    calls.append("bootout")
                },
                removeFile: { url in
                    XCTAssertEqual(url, expectedPlistURL)
                    calls.append("remove")
                }
            )
        )

        XCTAssertEqual(migrator.migrate(), .migrated(expectedPlistURL))
        XCTAssertEqual(calls.snapshot(), ["bootout", "remove"])
    }

    func testBootoutThrowsStopsRemoval() {
        struct BootoutError: Error, CustomStringConvertible {
            var description: String { "bootout exploded" }
        }

        let home = URL(fileURLWithPath: "/Users/tester", isDirectory: true)
        let expectedPlistURL = plistURL(home: home)
        let calls = CallRecorder()
        let migrator = LaunchAgentMigrator(
            dependencies: .init(
                homeDirectory: { home },
                fileExists: { _ in true },
                bootout: { _ in throw BootoutError() },
                removeFile: { _ in calls.append("remove") }
            )
        )

        XCTAssertEqual(migrator.migrate(), .failed(expectedPlistURL, "bootout exploded"))
        XCTAssertEqual(calls.snapshot(), [])
    }

    func testRemoveFileFailureAfterSuccessfulBootoutReportsFailure() {
        struct RemoveError: Error, CustomStringConvertible {
            var description: String { "permission denied" }
        }

        let home = URL(fileURLWithPath: "/Users/tester", isDirectory: true)
        let expectedPlistURL = plistURL(home: home)
        let calls = CallRecorder()
        let migrator = LaunchAgentMigrator(
            dependencies: .init(
                homeDirectory: { home },
                fileExists: { _ in true },
                bootout: { _ in calls.append("bootout") },
                removeFile: { _ in throw RemoveError() }
            )
        )

        XCTAssertEqual(migrator.migrate(), .failed(expectedPlistURL, "permission denied"))
        XCTAssertEqual(calls.snapshot(), ["bootout"])
    }

    func testStartupRunsMigrationBeforeSupervisorStart() {
        var events: [String] = []
        let startup = HelperStartup(
            migrateLaunchAgent: {
                events.append("migrate")
                return .notNeeded
            },
            startSupervisor: {
                events.append("start")
            },
            reportMigration: { result in
                events.append("report")
                XCTAssertEqual(result, .notNeeded)
            }
        )

        startup.start()

        XCTAssertEqual(events, ["migrate", "report", "start"])
    }
}
