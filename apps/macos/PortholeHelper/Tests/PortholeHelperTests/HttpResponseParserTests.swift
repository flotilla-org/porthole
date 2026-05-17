import XCTest
@testable import PortholeHelper

final class HttpResponseParserTests: XCTestCase {
    func testParsesSuccessfulJsonBody() throws {
        let response = """
        HTTP/1.1 200 OK\r
        content-type: application/json\r
        content-length: 11\r
        \r
        {"ok":true}
        """.data(using: .utf8)!

        let parsed = try HttpResponseParser.parse(response, maxBodyBytes: 1024)
        XCTAssertEqual(parsed.statusCode, 200)
        XCTAssertEqual(String(data: parsed.body, encoding: .utf8), "{\"ok\":true}")
    }

    func testRejectsOversizedBody() {
        let response = Data("HTTP/1.1 200 OK\r\n\r\n12345".utf8)
        XCTAssertThrowsError(try HttpResponseParser.parse(response, maxBodyBytes: 4))
    }

    func testPreservesNon2xxBodyForWireErrors() throws {
        let response = Data("HTTP/1.1 403 Forbidden\r\n\r\n{\"code\":\"system_permission_needed\"}".utf8)
        let parsed = try HttpResponseParser.parse(response, maxBodyBytes: 1024)
        XCTAssertEqual(parsed.statusCode, 403)
        XCTAssertEqual(String(data: parsed.body, encoding: .utf8), "{\"code\":\"system_permission_needed\"}")
    }
}
