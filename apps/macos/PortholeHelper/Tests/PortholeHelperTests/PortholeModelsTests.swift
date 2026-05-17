import XCTest
@testable import PortholeHelper

final class PortholeModelsTests: XCTestCase {
    func testInfoResponseDecodesSnakeCasePermissions() throws {
        let json = """
        {
          "daemon_version": "0.0.0",
          "uptime_seconds": 7,
          "adapters": [{
            "name": "macos",
            "loaded": true,
            "capabilities": ["system_permission_prompt"],
            "system_permissions": [{
              "name": "accessibility",
              "granted": false,
              "purpose": "input injection and some wait conditions"
            }]
          }]
        }
        """.data(using: .utf8)!

        let info = try PortholeJSON.decoder.decode(InfoResponse.self, from: json)
        XCTAssertEqual(info.daemonVersion, "0.0.0")
        XCTAssertEqual(info.uptimeSeconds, 7)
        XCTAssertEqual(info.adapters.first?.systemPermissions.first?.id, "accessibility")
        XCTAssertEqual(info.adapters.first?.systemPermissions.first?.purpose, "input injection and some wait conditions")
    }

    func testWireErrorDetailsArePreserved() throws {
        let json = """
        {
          "code": "system_permission_request_failed",
          "message": "prompt rejected by OS",
          "details": {
            "reason": "process is not running inside a .app bundle",
            "settings_path": "System Settings > Privacy & Security > Accessibility",
            "binary_path": "/Applications/Porthole.app/Contents/MacOS/portholed"
          }
        }
        """.data(using: .utf8)!

        let error = try PortholeJSON.decoder.decode(WireError.self, from: json)
        XCTAssertEqual(error.code, "system_permission_request_failed")
        XCTAssertEqual(error.details?["reason"]?.stringValue, "process is not running inside a .app bundle")
    }

    func testMissingSystemPermissionsDecodesAsEmpty() throws {
        let json = """
        {
          "daemon_version": "0.0.0",
          "uptime_seconds": 7,
          "adapters": [{
            "name": "memory",
            "loaded": true,
            "capabilities": []
          }]
        }
        """.data(using: .utf8)!

        let info = try PortholeJSON.decoder.decode(InfoResponse.self, from: json)
        XCTAssertEqual(info.adapters.first?.systemPermissions, [])
    }
}
