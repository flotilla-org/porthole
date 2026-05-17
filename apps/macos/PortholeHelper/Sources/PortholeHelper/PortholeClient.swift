import Foundation
import Network

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
        let operation = PortholeRawConnection(socketURL: socketURL, bytes: bytes)
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
    private let connection: NWConnection
    private let bytes: Data
    private let queue = DispatchQueue(label: "dev.porthole.helper.client")
    private var response = Data()
    private var continuation: CheckedContinuation<Data, Error>?
    private var didResume = false

    init(socketURL: URL, bytes: Data) {
        self.bytes = bytes
        self.connection = NWConnection(to: .unix(path: socketURL.path), using: .tcp)
    }

    func start() async throws -> Data {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                queue.async {
                    self.continuation = continuation
                    self.installHandlersAndStart()
                }
            }
        } onCancel: {
            cancel()
        }
    }

    func cancel() {
        queue.async {
            self.connection.cancel()
        }
    }

    private func installHandlersAndStart() {
        connection.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            self.queue.async {
                self.handle(state)
            }
        }
        connection.start(queue: queue)
    }

    private func handle(_ state: NWConnection.State) {
        switch state {
        case .ready:
            connection.send(content: bytes, completion: .contentProcessed { [weak self] error in
                guard let self else { return }
                self.queue.async {
                    if let error {
                        self.resume(.failure(PortholeClientError.connectionFailed(error.localizedDescription)))
                    } else {
                        self.receiveNext()
                    }
                }
            })
        case .failed(let error):
            resume(.failure(PortholeClientError.connectionFailed(error.localizedDescription)))
        default:
            break
        }
    }

    private func receiveNext() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            self.queue.async {
                if let data {
                    self.response.append(data)
                }
                if let error {
                    self.resume(.failure(PortholeClientError.connectionFailed(error.localizedDescription)))
                } else if isComplete {
                    self.resume(.success(self.response))
                } else {
                    self.receiveNext()
                }
            }
        }
    }

    private func resume(_ result: Result<Data, Error>) {
        guard !didResume else { return }
        didResume = true
        connection.cancel()
        continuation?.resume(with: result)
        continuation = nil
    }
}
