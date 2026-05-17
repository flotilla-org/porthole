import Foundation

struct ParsedHttpResponse: Equatable {
    let statusCode: Int
    let body: Data
}

enum HttpResponseParserError: Error, Equatable {
    case missingHeaderTerminator
    case malformedStatusLine
    case invalidStatusCode
    case bodyTooLarge(limit: Int)
}

enum HttpResponseParser {
    private static let headerTerminator = Data("\r\n\r\n".utf8)

    static func parse(_ data: Data, maxBodyBytes: Int) throws -> ParsedHttpResponse {
        guard let split = data.range(of: headerTerminator) else {
            throw HttpResponseParserError.missingHeaderTerminator
        }

        let header = data[..<split.lowerBound]
        let bodyStart = split.upperBound
        let body = data[bodyStart...]
        guard body.count <= maxBodyBytes else {
            throw HttpResponseParserError.bodyTooLarge(limit: maxBodyBytes)
        }

        guard let headerText = String(data: header, encoding: .utf8),
              let statusLine = headerText.split(separator: "\r\n").first
        else {
            throw HttpResponseParserError.malformedStatusLine
        }

        let parts = statusLine.split(separator: " ")
        guard parts.count >= 2 else {
            throw HttpResponseParserError.malformedStatusLine
        }
        guard let statusCode = Int(parts[1]) else {
            throw HttpResponseParserError.invalidStatusCode
        }

        return ParsedHttpResponse(statusCode: statusCode, body: Data(body))
    }
}
