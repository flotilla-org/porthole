import Foundation
import Darwin

protocol PortholeClientProtocol: Sendable {
    func info() async throws -> InfoResponse
    func requestPermissionPrompt(name: String) async throws -> SystemPermissionPromptOutcome
}

struct PortholeHttpRequest {
    let bytes: Data

    static func get(path: String) -> PortholeHttpRequest {
        let head = "GET \(path) HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        return PortholeHttpRequest(bytes: Data(head.utf8))
    }

    static func post(path: String, body: Data) -> PortholeHttpRequest {
        var head = ""
        head += "POST \(path) HTTP/1.1\r\n"
        head += "Host: localhost\r\n"
        head += "Connection: close\r\n"
        head += "Content-Type: application/json\r\n"
        head += "Content-Length: \(body.count)\r\n"
        head += "\r\n"
        var data = Data(head.utf8)
        data.append(body)
        return PortholeHttpRequest(bytes: data)
    }
}

enum PortholeClientError: Error, Equatable {
    case connectionFailed(String)
    case invalidResponse(String)
    case timedOut(TimeInterval)
    case httpError(statusCode: Int, wire: WireError?)
}

extension PortholeClientError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .connectionFailed(let message):
            "Could not connect to portholed: \(message)"
        case .invalidResponse(let message):
            "Invalid response from portholed: \(message)"
        case .timedOut(let seconds):
            "Timed out waiting for portholed after \(seconds) seconds"
        case .httpError(let statusCode, let wire):
            if let wire {
                "\(wire.message) (\(wire.code), HTTP \(statusCode))"
            } else {
                "portholed returned HTTP \(statusCode)"
            }
        }
    }
}

final class PortholeClient: PortholeClientProtocol, @unchecked Sendable {
    private let socketURL: URL
    private let requestTimeoutSeconds: TimeInterval
    private let maxBodyBytes = 1024 * 1024

    init(socketURL: URL = PortholeSocketPath.current(), requestTimeoutSeconds: TimeInterval = 5) {
        self.socketURL = socketURL
        self.requestTimeoutSeconds = requestTimeoutSeconds
    }

    func info() async throws -> InfoResponse {
        let body = try await send(.get(path: "/info"))
        return try PortholeJSON.decoder.decode(InfoResponse.self, from: body)
    }

    func requestPermissionPrompt(name: String) async throws -> SystemPermissionPromptOutcome {
        let encoded = try PortholeJSON.encoder.encode(SystemPermissionPromptRequest(name: name))
        let body = try await send(.post(path: "/system-permissions/request", body: encoded))
        return try PortholeJSON.decoder.decode(SystemPermissionPromptOutcome.self, from: body)
    }

    private func send(_ request: PortholeHttpRequest) async throws -> Data {
        let response = try await sendRaw(request.bytes)
        let parsed = try HttpResponseParser.parse(response, maxBodyBytes: maxBodyBytes)
        guard (200..<300).contains(parsed.statusCode) else {
            let wire = try? PortholeJSON.decoder.decode(WireError.self, from: parsed.body)
            throw PortholeClientError.httpError(statusCode: parsed.statusCode, wire: wire)
        }
        return parsed.body
    }

    private func sendRaw(_ bytes: Data) async throws -> Data {
        let operation = PortholeRawConnection(socketURL: socketURL, bytes: bytes, timeoutSeconds: requestTimeoutSeconds)
        let timeoutSeconds = requestTimeoutSeconds
        defer { operation.cancel() }

        return try await withThrowingTaskGroup(of: Data.self) { group in
            group.addTask {
                try await operation.start()
            }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(timeoutSeconds * 1_000_000_000))
                throw PortholeClientError.timedOut(timeoutSeconds)
            }

            do {
                guard let response = try await group.next() else {
                    throw PortholeClientError.invalidResponse("request completed without a response")
                }
                group.cancelAll()
                return response
            } catch {
                group.cancelAll()
                throw error
            }
        }
    }
}

private final class PortholeRawConnection: @unchecked Sendable {
    private let socketURL: URL
    private let bytes: Data
    private let timeoutSeconds: TimeInterval
    private let lock = NSLock()
    private var fd: Int32 = -1

    init(socketURL: URL, bytes: Data, timeoutSeconds: TimeInterval) {
        self.socketURL = socketURL
        self.bytes = bytes
        self.timeoutSeconds = timeoutSeconds
    }

    func start() async throws -> Data {
        try await withTaskCancellationHandler {
            try connectWriteRead()
        } onCancel: {
            cancel()
        }
    }

    func cancel() {
        let current: Int32 = lock.withLock {
            let current = fd
            fd = -1
            return current
        }
        if current >= 0 {
            Darwin.close(current)
        }
    }

    private func connectWriteRead() throws -> Data {
        let socketFD = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard socketFD >= 0 else {
            throw PortholeClientError.connectionFailed(errnoMessage(prefix: "socket"))
        }

        lock.withLock {
            fd = socketFD
        }
        defer { cancel() }

        try configureTimeouts(socketFD)
        try connect(socketFD)
        try writeAll(socketFD, bytes)
        return try readAll(socketFD)
    }

    private func configureTimeouts(_ socketFD: Int32) throws {
        var timeout = timeval(
            tv_sec: __darwin_time_t(timeoutSeconds.rounded(.up)),
            tv_usec: 0
        )
        let timeoutSize = socklen_t(MemoryLayout<timeval>.size)
        let readResult = withUnsafePointer(to: &timeout) { pointer in
            pointer.withMemoryRebound(to: UInt8.self, capacity: MemoryLayout<timeval>.size) {
                Darwin.setsockopt(socketFD, SOL_SOCKET, SO_RCVTIMEO, $0, timeoutSize)
            }
        }
        guard readResult == 0 else {
            throw PortholeClientError.connectionFailed(errnoMessage(prefix: "setsockopt(SO_RCVTIMEO)"))
        }
        let writeResult = withUnsafePointer(to: &timeout) { pointer in
            pointer.withMemoryRebound(to: UInt8.self, capacity: MemoryLayout<timeval>.size) {
                Darwin.setsockopt(socketFD, SOL_SOCKET, SO_SNDTIMEO, $0, timeoutSize)
            }
        }
        guard writeResult == 0 else {
            throw PortholeClientError.connectionFailed(errnoMessage(prefix: "setsockopt(SO_SNDTIMEO)"))
        }
    }

    private func connect(_ socketFD: Int32) throws {
        var addr = sockaddr_un()
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        addr.sun_family = sa_family_t(AF_UNIX)

        let pathBytes = Array(socketURL.path.utf8)
        let pathCapacity = MemoryLayout.size(ofValue: addr.sun_path)
        guard pathBytes.count < pathCapacity else {
            throw PortholeClientError.connectionFailed("socket path is too long: \(socketURL.path)")
        }

        withUnsafeMutablePointer(to: &addr.sun_path) { pointer in
            pointer.withMemoryRebound(to: CChar.self, capacity: pathCapacity) { chars in
                for index in 0..<pathBytes.count {
                    chars[index] = CChar(bitPattern: pathBytes[index])
                }
                chars[pathBytes.count] = 0
            }
        }

        let result = withUnsafePointer(to: &addr) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(socketFD, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard result == 0 else {
            throw PortholeClientError.connectionFailed("\(socketURL.path): \(errnoMessage(prefix: "connect"))")
        }
    }

    private func writeAll(_ socketFD: Int32, _ data: Data) throws {
        try data.withUnsafeBytes { rawBuffer in
            guard let base = rawBuffer.baseAddress else { return }
            var offset = 0
            while offset < rawBuffer.count {
                let written = Darwin.write(socketFD, base.advanced(by: offset), rawBuffer.count - offset)
                if written > 0 {
                    offset += written
                } else if written == -1 && errno == EINTR {
                    continue
                } else {
                    throw PortholeClientError.connectionFailed(errnoMessage(prefix: "write"))
                }
            }
        }
    }

    private func readAll(_ socketFD: Int32) throws -> Data {
        var response = Data()
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        while true {
            let readCount = Darwin.read(socketFD, &buffer, buffer.count)
            if readCount > 0 {
                response.append(buffer, count: readCount)
            } else if readCount == 0 {
                return response
            } else if errno == EINTR {
                continue
            } else if errno == EAGAIN || errno == EWOULDBLOCK {
                throw PortholeClientError.timedOut(timeoutSeconds)
            } else {
                throw PortholeClientError.connectionFailed(errnoMessage(prefix: "read"))
            }
        }
    }

    private func errnoMessage(prefix: String) -> String {
        "\(prefix): \(String(cString: strerror(errno)))"
    }
}
