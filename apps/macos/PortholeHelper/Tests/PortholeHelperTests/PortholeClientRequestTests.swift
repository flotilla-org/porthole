import XCTest
@testable import PortholeHelper

final class PortholeClientRequestTests: XCTestCase {
    func testSocketTimeoutPreservesFractionalSeconds() {
        let timeout = PortholeSocketTimeout.makeTimeval(for: 2.3)

        XCTAssertEqual(timeout.tv_sec, 2)
        XCTAssertEqual(timeout.tv_usec, 300_000)
    }

    func testUnixSocketAddressLengthIncludesPathTerminator() {
        let path = "/tmp/porthole.sock"
        let length = PortholeUnixSocketAddress.length(pathByteCount: path.utf8.count)

        XCTAssertEqual(length, socklen_t(2 + path.utf8.count + 1))
    }

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
