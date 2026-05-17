# macOS Onboarding UI Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a native macOS onboarding window to `PortholeHelper` that shows system-permission state from `/info`, requests prompts through `/system-permissions/request`, opens System Settings, restarts/verifies the daemon when needed, and preserves the CLI's one-permission-at-a-time onboarding semantics.

**Architecture:** Keep daemon-owned truth and policy in Rust. Add a small Swift UDS HTTP client, pure Swift onboarding reducer/state machine with tests, and a thin AppKit window launched from the existing menu-bar helper. The UI presents daemon state and calls existing daemon APIs; it does not add new daemon routes or permission workarounds.

**Tech Stack:** SwiftPM executable and XCTest target, AppKit, Network.framework Unix-domain connections, Foundation JSON decoding, existing Rust daemon `/info` and `/system-permissions/request`, existing `cargo xtask bundle --platform macos` packaging.

---

## Design Inputs

- Spec: `docs/superpowers/specs/2026-05-17-macos-onboarding-ui-design.md`
- Roadmap item: Phase 3 `Onboard UI flow - native equivalent of porthole onboard`
- Existing helper: `apps/macos/PortholeHelper/Sources/PortholeHelper/`
- Existing daemon APIs:
  - `GET /info`
  - `POST /system-permissions/request`
- Existing repo gates:
  - `cargo build --workspace --locked`
  - `cargo test --workspace --locked`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo +nightly-2026-03-12 fmt --check`

## Permission-Sensitive Rule

This slice must not add a mock path, skip path, feature flag, or degraded success path for real macOS permissions. Automated tests cover pure Swift logic and daemon client parsing. Manual UI smoke may open the onboarding window and click non-permission controls. If live Accessibility or Screen Recording grants are missing, stop `BLOCKED`, state the missing permission, and wait for the user to grant it.

## File Structure

- Modify `apps/macos/PortholeHelper/Package.swift`
  - Add a test target.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeModels.swift`
  - Swift Decodable models for `/info`, permission rows, prompt outcomes, and wire errors.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeSocketPath.swift`
  - Mirror Rust socket path resolution.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/HttpResponseParser.swift`
  - Pure parser for the tiny HTTP client and 1 MiB body cap.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeClient.swift`
  - Network.framework UDS HTTP client and protocol for fakes.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingFlow.swift`
  - Pure state machine/reducer.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/SettingsLinks.swift`
  - Known Settings URLs and text-only fallback for unknown permission names.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingWindowController.swift`
  - AppKit UI and action wiring.
- Modify `apps/macos/PortholeHelper/Sources/PortholeHelper/DaemonSupervisor.swift`
  - Expose current state and helper-owned restart capability for onboarding.
- Modify `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift`
  - Add `Open Onboarding...` menu item and window lifetime.
- Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/*.swift`
  - XCTest coverage for models, socket path, parser, settings links, and state machine.
- Modify `docs/roadmap.md`
  - Tick only the onboarding UI implementation item after all gates and smoke pass.

---

## Chunk 1: SwiftPM Test Harness And Pure Models

### Task 1: Add XCTest Target And Model Decoding

**Files:**
- Modify: `apps/macos/PortholeHelper/Package.swift`
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeModels.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeModelsTests.swift`

- [ ] **Step 1: Add a failing model decoding test**

Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeModelsTests.swift`:

```swift
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter PortholeModelsTests
```

Expected: FAIL because the package has no test target and models do not exist.

- [ ] **Step 3: Add the test target**

Modify `apps/macos/PortholeHelper/Package.swift`:

```swift
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "PortholeHelper",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "PortholeHelper", targets: ["PortholeHelper"]),
    ],
    targets: [
        .executableTarget(name: "PortholeHelper"),
        .testTarget(name: "PortholeHelperTests", dependencies: ["PortholeHelper"]),
    ]
)
```

- [ ] **Step 4: Add the model implementation**

Create `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeModels.swift`:

```swift
import Foundation

enum PortholeJSON {
    static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }()
}

struct InfoResponse: Decodable, Equatable {
    let daemonVersion: String
    let uptimeSeconds: UInt64
    let adapters: [AdapterInfo]
}

struct AdapterInfo: Decodable, Equatable {
    let name: String
    let loaded: Bool
    let capabilities: [String]
    let systemPermissions: [SystemPermissionStatus]

    enum CodingKeys: String, CodingKey {
        case name
        case loaded
        case capabilities
        case systemPermissions
    }

    init(name: String, loaded: Bool, capabilities: [String], systemPermissions: [SystemPermissionStatus]) {
        self.name = name
        self.loaded = loaded
        self.capabilities = capabilities
        self.systemPermissions = systemPermissions
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.decode(String.self, forKey: .name)
        loaded = try container.decode(Bool.self, forKey: .loaded)
        capabilities = try container.decode([String].self, forKey: .capabilities)
        systemPermissions = try container.decodeIfPresent([SystemPermissionStatus].self, forKey: .systemPermissions) ?? []
    }
}

struct SystemPermissionStatus: Decodable, Equatable, Identifiable {
    let name: String
    let granted: Bool
    let purpose: String

    var id: String { name }
}

struct SystemPermissionPromptRequest: Encodable, Equatable {
    let name: String
}

struct SystemPermissionPromptOutcome: Decodable, Equatable {
    let permission: String
    let grantedBefore: Bool
    let grantedAfter: Bool
    let requiresDaemonRestart: Bool
    let notes: String
}

enum JSONValue: Decodable, Equatable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: JSONValue])
    case array([JSONValue])
    case null

    var stringValue: String? {
        if case .string(let value) = self { value } else { nil }
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            self = .array(try container.decode([JSONValue].self))
        }
    }
}

struct WireError: Decodable, Error, Equatable {
    let code: String
    let message: String
    let details: [String: JSONValue]?
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter PortholeModelsTests
```

Expected: PASS.

- [ ] **Step 6: Commit**

```sh
git add apps/macos/PortholeHelper/Package.swift \
        apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeModels.swift \
        apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeModelsTests.swift
git commit -m "test: add helper protocol model decoding"
```

---

## Chunk 2: Socket Path And HTTP Client

### Task 2: Add Socket Path Resolution

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeSocketPath.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeSocketPathTests.swift`

- [ ] **Step 1: Add failing socket path tests**

Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeSocketPathTests.swift`:

```swift
import XCTest
@testable import PortholeHelper

final class PortholeSocketPathTests: XCTestCase {
    func testPortholeRuntimeDirWins() {
        let path = PortholeSocketPath.resolve(environment: [
            "PORTHOLE_RUNTIME_DIR": "/tmp/custom",
            "XDG_RUNTIME_DIR": "/tmp/xdg",
            "TMPDIR": "/tmp/trailing/"
        ], uid: 501)

        XCTAssertEqual(path.path, "/tmp/custom/porthole.sock")
    }

    func testXdgRuntimeDirIsSecondChoice() {
        let path = PortholeSocketPath.resolve(environment: [
            "XDG_RUNTIME_DIR": "/tmp/xdg"
        ], uid: 501)

        XCTAssertEqual(path.path, "/tmp/xdg/porthole/porthole.sock")
    }

    func testTmpdirIsStandardized() {
        let path = PortholeSocketPath.resolve(environment: [
            "TMPDIR": "/tmp/trailing/"
        ], uid: 501)

        XCTAssertEqual(path.path, "/tmp/trailing/porthole-501/porthole.sock")
    }

    func testDefaultFallsBackToTmp() {
        let path = PortholeSocketPath.resolve(environment: [:], uid: 501)
        XCTAssertEqual(path.path, "/tmp/porthole-501/porthole.sock")
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter PortholeSocketPathTests
```

Expected: FAIL because `PortholeSocketPath` does not exist.

- [ ] **Step 3: Implement socket path resolution**

Create `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeSocketPath.swift`:

```swift
import Foundation

enum PortholeSocketPath {
    static func current() -> URL {
        resolve(environment: ProcessInfo.processInfo.environment, uid: getuid())
    }

    static func resolve(environment: [String: String], uid: uid_t) -> URL {
        if let dir = environment["PORTHOLE_RUNTIME_DIR"], !dir.isEmpty {
            return standardized(dir, ["porthole.sock"])
        }
        if let dir = environment["XDG_RUNTIME_DIR"], !dir.isEmpty {
            return standardized(dir, ["porthole", "porthole.sock"])
        }
        if let tmp = environment["TMPDIR"], !tmp.isEmpty {
            return standardized(tmp, ["porthole-\(uid)", "porthole.sock"])
        }
        return standardized("/tmp", ["porthole-\(uid)", "porthole.sock"])
    }

    private static func standardized(_ root: String, _ components: [String]) -> URL {
        var url = URL(fileURLWithPath: root, isDirectory: true)
        for component in components {
            url.appendPathComponent(component)
        }
        return url.standardized
    }
}
```

- [ ] **Step 4: Run socket tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter PortholeSocketPathTests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeSocketPath.swift \
        apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeSocketPathTests.swift
git commit -m "feat: resolve helper daemon socket path"
```

### Task 3: Add Testable HTTP Response Parsing

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/HttpResponseParser.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/HttpResponseParserTests.swift`

- [ ] **Step 1: Add failing parser tests**

Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/HttpResponseParserTests.swift`:

```swift
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
```

- [ ] **Step 2: Run parser tests to verify failure**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter HttpResponseParserTests
```

Expected: FAIL because `HttpResponseParser` does not exist.

- [ ] **Step 3: Implement parser**

Create `apps/macos/PortholeHelper/Sources/PortholeHelper/HttpResponseParser.swift`:

```swift
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
    static func parse(_ data: Data, maxBodyBytes: Int) throws -> ParsedHttpResponse {
        guard let split = data.range(of: Data([13, 10, 13, 10])) else {
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
```

- [ ] **Step 4: Run parser tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter HttpResponseParserTests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/HttpResponseParser.swift \
        apps/macos/PortholeHelper/Tests/PortholeHelperTests/HttpResponseParserTests.swift
git commit -m "test: cover helper HTTP response parsing"
```

### Task 4: Add UDS HTTP Client

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeClient.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeClientRequestTests.swift`

- [ ] **Step 1: Add request-building tests**

Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeClientRequestTests.swift`:

```swift
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
```

- [ ] **Step 2: Run request tests to verify failure**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter PortholeClientRequestTests
```

Expected: FAIL because `PortholeHttpRequest` does not exist.

- [ ] **Step 3: Implement client protocol, request builder, and Network client**

Create `apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeClient.swift`:

```swift
import Foundation
import Network

protocol PortholeClientProtocol {
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

final class PortholeClient: PortholeClientProtocol {
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
```

Keep `PortholeRawConnection` isolated behind `PortholeClient`. It deliberately
serializes `NWConnection` callbacks onto its own queue, cancels when the outer
task is cancelled or times out, and is marked `@unchecked Sendable` because the
serialized queue owns the mutable `response` and continuation state. Do not make
the UI depend on Network.framework directly.

- [ ] **Step 4: Run client request tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter PortholeClientRequestTests
```

Expected: PASS.

- [ ] **Step 5: Run all helper tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper
```

Expected: PASS.

- [ ] **Step 6: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/PortholeClient.swift \
        apps/macos/PortholeHelper/Tests/PortholeHelperTests/PortholeClientRequestTests.swift
git commit -m "feat: add helper daemon client"
```

---

## Chunk 3: Onboarding State Machine

### Task 5: Add Settings Link Mapping

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/SettingsLinks.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/SettingsLinksTests.swift`

- [ ] **Step 1: Add failing settings-link tests**

Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/SettingsLinksTests.swift`:

```swift
import XCTest
@testable import PortholeHelper

final class SettingsLinksTests: XCTestCase {
    func testKnownPermissionHasDeepLink() {
        let link = SettingsLinks.link(for: "accessibility")
        XCTAssertEqual(link?.absoluteString, "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
    }

    func testUnknownPermissionHasNoGuessedDeepLink() {
        XCTAssertNil(SettingsLinks.link(for: "future_permission"))
    }

    func testDisplayNameFallback() {
        XCTAssertEqual(SettingsLinks.displayName(for: "screen_recording"), "Screen Recording")
        XCTAssertEqual(SettingsLinks.displayName(for: "future_permission"), "future permission")
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter SettingsLinksTests
```

Expected: FAIL because `SettingsLinks` does not exist.

- [ ] **Step 3: Implement settings links**

Create `apps/macos/PortholeHelper/Sources/PortholeHelper/SettingsLinks.swift`:

```swift
import Foundation

enum SettingsLinks {
    static func link(for permission: String) -> URL? {
        switch permission {
        case "accessibility":
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        case "screen_recording":
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        default:
            nil
        }
    }

    static func displayName(for permission: String) -> String {
        switch permission {
        case "accessibility":
            "Accessibility"
        case "screen_recording":
            "Screen Recording"
        default:
            permission.replacingOccurrences(of: "_", with: " ")
        }
    }
}
```

- [ ] **Step 4: Run settings-link tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter SettingsLinksTests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/SettingsLinks.swift \
        apps/macos/PortholeHelper/Tests/PortholeHelperTests/SettingsLinksTests.swift
git commit -m "feat: map onboarding settings links"
```

### Task 6: Add Pure Onboarding Reducer

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingFlow.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/OnboardingFlowTests.swift`

- [ ] **Step 1: Add failing state-machine tests**

Create `apps/macos/PortholeHelper/Tests/PortholeHelperTests/OnboardingFlowTests.swift`:

```swift
import XCTest
@testable import PortholeHelper

final class OnboardingFlowTests: XCTestCase {
    func testAllGrantedCompletesOnLoad() {
        let info = OnboardingFixtures.info([
            .init(name: "accessibility", granted: true, purpose: "input")
        ])
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(info))
        XCTAssertEqual(flow.state, .complete(info))
    }

    func testFirstMissingPermissionBecomesActive() {
        let info = OnboardingFixtures.info([
            .init(name: "accessibility", granted: false, purpose: "input"),
            .init(name: "screen_recording", granted: false, purpose: "capture")
        ])
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(info))
        XCTAssertEqual(flow.state.activePermissionName, "accessibility")
    }

    func testRequestOutcomeWaitsForUser() {
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(OnboardingFixtures.info([
            .init(name: "accessibility", granted: false, purpose: "input")
        ])))
        flow.apply(.requestStarted("accessibility"))
        flow.apply(.requestSucceeded(OnboardingFixtures.outcome("accessibility", requiresRestart: true)))
        XCTAssertEqual(flow.state, .waitingForUser(permission: "accessibility", outcome: OnboardingFixtures.outcome("accessibility", requiresRestart: true)))
    }

    func testLoadFailureBlocksWithMessage() {
        var flow = OnboardingFlow()
        flow.apply(.loadStarted)
        flow.apply(.loadFailed("daemon unavailable"))
        XCTAssertEqual(flow.state, .blocked(message: "daemon unavailable"))
    }

    func testVerificationSuccessAdvancesToNextPermission() {
        var flow = OnboardingFlow()
        flow.apply(.loadedInfo(OnboardingFixtures.info([
            .init(name: "accessibility", granted: false, purpose: "input")
        ])))
        flow.apply(.userConfirmedGrant("accessibility", requiresRestart: false))
        flow.apply(.verificationSucceeded(OnboardingFixtures.info([
            .init(name: "accessibility", granted: true, purpose: "input"),
            .init(name: "screen_recording", granted: false, purpose: "capture")
        ])))
        XCTAssertEqual(flow.state.activePermissionName, "screen_recording")
    }

    func testVerificationFailureBlocksWithMessage() {
        var flow = OnboardingFlow()
        flow.apply(.verificationFailed("permission still missing"))
        XCTAssertEqual(flow.state, .blocked(message: "permission still missing"))
    }

    func testRestartTimeoutBlocksWithManualControls() {
        var flow = OnboardingFlow()
        flow.apply(.restartTimedOut("accessibility"))
        XCTAssertEqual(flow.state, .blocked(message: "Daemon failed to restart while verifying accessibility"))
    }
}

enum OnboardingFixtures {
    static func info(_ permissions: [SystemPermissionStatus]) -> InfoResponse {
        InfoResponse(
            daemonVersion: "test",
            uptimeSeconds: 1,
            adapters: [AdapterInfo(name: "macos", loaded: true, capabilities: ["system_permission_prompt"], systemPermissions: permissions)]
        )
    }

    static func outcome(_ permission: String, requiresRestart: Bool) -> SystemPermissionPromptOutcome {
        SystemPermissionPromptOutcome(
            permission: permission,
            grantedBefore: false,
            grantedAfter: false,
            requiresDaemonRestart: requiresRestart,
            notes: "notes"
        )
    }
}
```

- [ ] **Step 2: Run reducer tests to verify failure**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter OnboardingFlowTests
```

Expected: FAIL because `OnboardingFlow` does not exist.

- [ ] **Step 3: Implement reducer**

Create `apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingFlow.swift`:

```swift
import Foundation

struct OnboardingFlow {
    enum State: Equatable {
        case idle
        case loadingInfo
        case ready(info: InfoResponse, activePermission: SystemPermissionStatus?)
        case requesting(permission: String)
        case waitingForUser(permission: String, outcome: SystemPermissionPromptOutcome)
        case restarting(permission: String)
        case verifying(permission: String)
        case complete(InfoResponse)
        case blocked(message: String)

        var activePermissionName: String? {
            switch self {
            case .ready(_, let active):
                active?.name
            case .requesting(let permission),
                 .waitingForUser(let permission, _),
                 .restarting(let permission),
                 .verifying(let permission):
                permission
            default:
                nil
            }
        }
    }

    enum Event: Equatable {
        case loadStarted
        case loadedInfo(InfoResponse)
        case loadFailed(String)
        case requestStarted(String)
        case requestSucceeded(SystemPermissionPromptOutcome)
        case requestFailed(String)
        case userConfirmedGrant(String, requiresRestart: Bool)
        case verificationSucceeded(InfoResponse)
        case verificationFailed(String)
        case restartTimedOut(String)
    }

    private(set) var state: State = .idle

    mutating func apply(_ event: Event) {
        switch event {
        case .loadStarted:
            state = .loadingInfo
        case .loadedInfo(let info):
            applyLoadedInfo(info)
        case .loadFailed(let message):
            state = .blocked(message: message)
        case .requestStarted(let permission):
            state = .requesting(permission: permission)
        case .requestSucceeded(let outcome):
            state = .waitingForUser(permission: outcome.permission, outcome: outcome)
        case .requestFailed(let message):
            state = .blocked(message: message)
        case .userConfirmedGrant(let permission, let requiresRestart):
            state = requiresRestart ? .restarting(permission: permission) : .verifying(permission: permission)
        case .verificationSucceeded(let info):
            applyLoadedInfo(info)
        case .verificationFailed(let message):
            state = .blocked(message: message)
        case .restartTimedOut(let permission):
            state = .blocked(message: "Daemon failed to restart while verifying \(permission)")
        }
    }

    private mutating func applyLoadedInfo(_ info: InfoResponse) {
        let missing = info.adapters.flatMap(\.systemPermissions).first { !$0.granted }
        if let missing {
            state = .ready(info: info, activePermission: missing)
        } else {
            state = .complete(info)
        }
    }
}
```

- [ ] **Step 4: Run reducer tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper --filter OnboardingFlowTests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingFlow.swift \
        apps/macos/PortholeHelper/Tests/PortholeHelperTests/OnboardingFlowTests.swift
git commit -m "feat: model onboarding state"
```

---

## Chunk 4: Supervisor Hooks And AppKit Window

### Task 7: Expose Daemon Ownership And Restart Outcome

**Files:**
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/DaemonSupervisor.swift`

- [ ] **Step 1: Add helper-owned restart API**

Modify `DaemonSupervisor` to keep `private(set) var currentState: State = .stopped` and publish all state changes through a single `setState(_:)` helper:

```swift
private(set) var currentState: State = .stopped

private func setState(_ state: State) {
    currentState = state
    onStateChange(state)
}
```

Replace direct `onStateChange(...)` calls with `setState(...)`.

Add:

```swift
enum RestartCapability: Equatable {
    case helperOwned
    case external
}

var restartCapability: RestartCapability {
    switch currentState {
    case .running:
        .helperOwned
    case .runningExternal:
        .external
    case .stopped, .crashed:
        .helperOwned
    }
}
```

These cases match the current `DaemonSupervisor.State` shape in the helper:
`.stopped`, `.running(pid:)`, `.runningExternal`, and `.crashed(status:)`.

The onboarding UI uses this to decide whether it can restart directly or must
show manual restart instructions.

- [ ] **Step 2: Build Swift helper**

Run:

```sh
swift build --package-path apps/macos/PortholeHelper --product PortholeHelper --scratch-path target/swift/PortholeHelper -c debug
```

Expected: PASS.

- [ ] **Step 3: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/DaemonSupervisor.swift
git commit -m "feat: expose helper daemon restart capability"
```

### Task 8: Add Onboarding Window Controller

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingWindowController.swift`
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift`

- [ ] **Step 1: Add window controller**

Create `OnboardingWindowController.swift` with an AppKit-first layout. Keep it
programmatic and restrained:

```swift
import AppKit

@MainActor
final class OnboardingWindowController: NSWindowController {
    private let client: PortholeClientProtocol
    private let supervisor: DaemonSupervisor
    private var flow = OnboardingFlow()

    private let statusLabel = NSTextField(labelWithString: "Checking daemon...")
    private let permissionsStack = NSStackView()
    private let detailLabel = NSTextField(wrappingLabelWithString: "")
    private let primaryButton = NSButton(title: "Request Permission", target: nil, action: nil)
    private let settingsButton = NSButton(title: "Open Settings", target: nil, action: nil)
    private let refreshButton = NSButton(title: "Refresh", target: nil, action: nil)
    private let restartButton = NSButton(title: "Restart Daemon", target: nil, action: nil)

    init(client: PortholeClientProtocol = PortholeClient(), supervisor: DaemonSupervisor) {
        self.client = client
        self.supervisor = supervisor
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 420),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Porthole Onboarding"
        super.init(window: window)
        installContent()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}
```

Implement these methods in the same file:

- `installContent()` creates a vertical stack with header, status, permission
  rows, detail label, and footer buttons. Set explicit `accessibilityLabel`
  values on buttons and dynamic labels because the layout is programmatic.
- `refresh()` applies `.loadStarted`, calls `client.info()`, then applies
  `.loadedInfo` or `.loadFailed(error.localizedDescription)`.
- `requestActivePermission()` calls `client.requestPermissionPrompt(name:)`.
- `confirmGranted()` handles restart/verifying based on the last outcome.
- `restartDaemonForOnboarding(permission:)` checks `supervisor.restartCapability`.
- `waitForInfo(timeoutSeconds:)` polls `client.info()` for up to 10 seconds and
  applies `.verificationSucceeded(info)` when `/info` returns or
  `.verificationFailed(...)` when the deadline expires.
- `render()` updates labels/buttons from `flow.state`.

Keep button titles short and action-oriented. Do not add instructional copy
outside the current permission's concrete state.

- [ ] **Step 2: Wire menu item and window lifetime**

Modify `AppDelegate.swift`:

```swift
private var onboardingWindowController: OnboardingWindowController?
```

Add menu item after the status separator:

```swift
menu.addItem(NSMenuItem(title: "Open Onboarding...", action: #selector(openOnboarding), keyEquivalent: "o"))
```

Add action:

```swift
@objc private func openOnboarding() {
    guard let supervisor else { return }
    let controller = onboardingWindowController ?? OnboardingWindowController(supervisor: supervisor)
    onboardingWindowController = controller
    controller.showWindow(nil)
    if #available(macOS 14.0, *) {
        NSApp.activate()
    } else {
        NSApp.activate(ignoringOtherApps: true)
    }
}
```

- [ ] **Step 3: Build Swift helper**

Run:

```sh
swift build --package-path apps/macos/PortholeHelper --product PortholeHelper --scratch-path target/swift/PortholeHelper -c debug
```

Expected: PASS.

- [ ] **Step 4: Commit**

```sh
git add apps/macos/PortholeHelper/Sources/PortholeHelper/OnboardingWindowController.swift \
        apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift
git commit -m "feat: add macOS onboarding window"
```

---

## Chunk 5: Packaging, Docs, And Verification

### Task 9: Bundle Smoke And Roadmap

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Run all helper tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper
```

Expected: PASS.

- [ ] **Step 2: Run bundle smoke**

Run:

```sh
./scripts/tests/test-dev-bundle.sh
```

Expected: PASS and `dev-bundle: ok`.

- [ ] **Step 3: Optional non-permission GUI smoke**

Run:

```sh
open target/debug/Porthole.app
sleep 2
pgrep -fl PortholeHelper
osascript -e 'tell application id "org.flotilla.porthole.dev" to quit'
```

Expected: `PortholeHelper` starts and quits cleanly. If this machine cannot open
GUI apps, record that limitation. Do not substitute live permission/capture
tests.

- [ ] **Step 4: Update roadmap**

Modify `docs/roadmap.md` and tick only:

```text
Onboard UI flow - native equivalent of `porthole onboard`.
```

Do not tick notification approvals, `SMAppService`, LaunchAgent migration, or
external-daemon passive recovery.

- [ ] **Step 5: Run required repo gates**

Run:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all PASS. Permission-gated ignored tests remain ignored.

- [ ] **Step 6: Commit final docs**

```sh
git add docs/roadmap.md
git commit -m "docs: mark macOS onboarding UI complete"
```

### Task 10: PR Description Checklist

Before opening the PR, include:

- Summary of new Swift client, reducer, and window.
- Explicit note that no new daemon endpoint was added.
- Explicit note that real TCC grant flow was not automated.
- Validation commands:
  - `swift test --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper`
  - `swift build --package-path apps/macos/PortholeHelper --product PortholeHelper --scratch-path target/swift/PortholeHelper -c debug`
  - `./scripts/tests/test-dev-bundle.sh`
  - required repo gates from `AGENTS.md`
- Manual smoke result or reason it could not be run.

Do not claim Accessibility or Screen Recording grant behavior was verified unless
you actually ran an installed-bundle permission smoke with grants present.
