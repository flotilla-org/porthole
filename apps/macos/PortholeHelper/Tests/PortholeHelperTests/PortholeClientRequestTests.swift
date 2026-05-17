import XCTest
@testable import PortholeHelper

final class PortholeClientRequestTests: XCTestCase {
    func testBuildsGetInfoRequest() throws {
        let request = PortholeHttpRequest.get(path: "/info")
        let text = String(data: request.bytes, encoding: .utf8)!
        XCTAssertTrue(text.hasPrefix("GET /info HTTP/1.1\r\n"))
        XCTAssertTrue(text.contains("Host: localhost\r\n"))
        XCTAssertTrue(text.hasSuffix("\r\n\r\n"))
    }

    func testBuildsJsonPostRequest() throws {
        let body = try PortholeJSON.encoder.encode(SystemPermissionPromptRequest(name: "accessibility"))
        let request = PortholeHttpRequest.post(path: "/system-permissions/request", body: body)
        let text = String(data: request.bytes, encoding: .utf8)!
        XCTAssertTrue(text.hasPrefix("POST /system-permissions/request HTTP/1.1\r\n"))
        XCTAssertTrue(text.contains("Content-Type: application/json\r\n"))
        XCTAssertTrue(text.contains("Content-Length: \(body.count)\r\n"))
        XCTAssertTrue(text.hasSuffix("\r\n\r\n{\"name\":\"accessibility\"}"))
    }
}
