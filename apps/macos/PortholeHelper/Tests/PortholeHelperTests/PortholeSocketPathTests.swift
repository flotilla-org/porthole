import XCTest
@testable import PortholeHelper

final class PortholeSocketPathTests: XCTestCase {
    func testPortholeRuntimeDirWins() {
        let path = PortholeSocketPath.resolve(environment: [
            "PORTHOLE_RUNTIME_DIR": "/tmp/custom",
            "XDG_RUNTIME_DIR": "/tmp/xdg",
            "TMPDIR": "/tmp/trailing/",
        ], uid: 501)

        XCTAssertEqual(path.path, "/tmp/custom/porthole.sock")
    }

    func testXdgRuntimeDirIsSecondChoice() {
        let path = PortholeSocketPath.resolve(environment: [
            "XDG_RUNTIME_DIR": "/tmp/xdg",
        ], uid: 501)

        XCTAssertEqual(path.path, "/tmp/xdg/porthole/porthole.sock")
    }

    func testTmpdirIsStandardized() {
        let path = PortholeSocketPath.resolve(environment: [
            "TMPDIR": "/tmp/trailing/",
        ], uid: 501)

        XCTAssertEqual(path.path, "/tmp/trailing/porthole-501/porthole.sock")
    }

    func testDefaultFallsBackToTmp() {
        let path = PortholeSocketPath.resolve(environment: [:], uid: 501)
        XCTAssertEqual(path.path, "/tmp/porthole-501/porthole.sock")
    }
}
