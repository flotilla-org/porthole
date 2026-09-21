// THROWAWAY authorization integration probe; no privileged operation is executed.
import Foundation
import CryptoKit
import LocalAuthentication
import Security
import Darwin

let right = "work.flotilla.prototype.remote-approval"
let protocolName = "porthole-authorization-prototype-v1"
let defaultSocket = "/var/run/porthole-auth-prototype/broker.sock"
struct Failure: Error, CustomStringConvertible { let description: String; init(_ text: String) { description = text } }
struct Challenge: Codable {
    let protocolName: String
    let nonce: String
    let host: String
    let right: String
    let expires: Double
}
struct Decision: Codable { let payload: String; let decision: String; let signature: String }
struct KeyFile: Codable { let kind: String; let wrappedKey: String }
struct Message: Codable { let command: String; var response: Decision? = nil }
func encoded<T: Encodable>(_ value: T) throws -> Data {
    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
    return try encoder.encode(value)
}
func decoded<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T { try JSONDecoder().decode(type, from: data) }
func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
func publish<T: Encodable>(_ value: T) throws { FileHandle.standardOutput.write(try encoded(value) + Data([10])) }
func bytes(_ text: String) throws -> Data {
    guard let data = Data(base64Encoded: text) else { throw Failure("invalid base64") }; return data
}
func signedBytes(_ response: Decision) -> Data { Data("\(protocolName)\n\(response.decision)\n\(response.payload)".utf8) }
func makeSocket(_ path: String, server: Bool) throws -> Int32 {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { throw Failure("socket: \(errno)") }
    do {
        var noSignal: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSignal, socklen_t(MemoryLayout.size(ofValue: noSignal)))
        var timeout = timeval(tv_sec: 65, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout)))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout)))
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        guard path.utf8.count < MemoryLayout.size(ofValue: address.sun_path) else { throw Failure("socket path too long") }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            buffer.copyBytes(from: Array(path.utf8) + [0])
        }
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                server ? Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) : Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard result == 0 else { throw Failure("\(server ? "bind" : "connect"): \(errno)") }
        if server {
            guard chmod(path, 0o666) == 0, listen(fd, 8) == 0 else { throw Failure("listen: \(errno)") }
        }
        return fd
    } catch { close(fd); throw error }
}
func sendLine(_ fd: Int32, _ data: Data) throws {
    var bytes = data + Data([10])
    while !bytes.isEmpty {
        let count = bytes.withUnsafeBytes { Darwin.send(fd, $0.baseAddress, $0.count, 0) }
        guard count > 0 else { throw Failure("send: \(errno)") }; bytes.removeFirst(count)
    }
}
func readLine(_ fd: Int32) throws -> Data {
    var data = Data(); var byte: UInt8 = 0
    while data.count < 16384 {
        guard recv(fd, &byte, 1, 0) == 1 else { throw Failure("connection closed or timed out") }
        if byte == 10 { return data }; data.append(byte)
    }
    throw Failure("oversized message")
}
func exchange(_ message: Message, _ path: String) throws -> Data {
    let fd = try makeSocket(path, server: false); defer { close(fd) }
    try sendLine(fd, encoded(message)); return try readLine(fd)
}
final class Pending {
    let payload: String
    let expires: Double
    var decision: String? = nil
    init(payload: String, expires: Double) { self.payload = payload; self.expires = expires }
}
final class Broker {
    let publicKey: P256.Signing.PublicKey
    let automatic: Bool
    let permittedRequestUID: uid_t
    let lock = NSLock()
    var pending: [String: Pending] = [:]
    init(key: Data, automatic: Bool, demo: Bool) throws {
        publicKey = try P256.Signing.PublicKey(x963Representation: key)
        self.automatic = automatic
        permittedRequestUID = demo ? geteuid() : 0
        if !demo && geteuid() != 0 { throw Failure("real mechanism broker must run as root") }
    }
    func handle(_ fd: Int32) {
        defer { close(fd) }
        do {
            let message = try decoded(Message.self, readLine(fd))
            switch message.command {
            case "request":
                var uid: uid_t = 0; var gid: gid_t = 0
                guard getpeereid(fd, &uid, &gid) == 0, uid == permittedRequestUID else { throw Failure("request peer is not the mechanism host") }
                if automatic {
                    note("POLICY ALLOW right=\(right) peer_uid=\(uid); custom probe only")
                    try sendLine(fd, Data("ALLOW".utf8)); return
                }
                let expires = Date().timeIntervalSince1970 + 60
                var nonce = Data(count: 32)
                let status = nonce.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, $0.count, $0.baseAddress!) }
                guard status == errSecSuccess else { throw Failure("nonce generation failed") }
                let challenge = Challenge(protocolName: protocolName, nonce: nonce.base64EncodedString(), host: ProcessInfo.processInfo.hostName, right: right, expires: expires)
                let payload = try encoded(challenge).base64EncodedString()
                let entry = Pending(payload: payload, expires: expires)
                lock.lock(); pending[payload] = entry; lock.unlock()
                note("PENDING \(String(data: try encoded(challenge), encoding: .utf8)!)")
                defer { lock.lock(); pending.removeValue(forKey: payload); lock.unlock() }
                while Date().timeIntervalSince1970 < expires {
                    lock.lock(); let answer = entry.decision; lock.unlock()
                    if let answer {
                        note("HUMAN \(answer) nonce=\(challenge.nonce)")
                        try sendLine(fd, Data((answer == "allow" ? "ALLOW" : "DENY").utf8)); return
                    }
                    var pfd = pollfd(fd: fd, events: Int16(POLLIN | POLLHUP), revents: 0)
                    if poll(&pfd, 1, 100) != 0 { note("CANCELLED nonce=\(challenge.nonce)"); return }
                }
                note("EXPIRED nonce=\(challenge.nonce)"); try sendLine(fd, Data("DENY".utf8))
            case "pending":
                lock.lock(); let entries = pending.keys.sorted(); lock.unlock()
                try sendLine(fd, encoded(entries))
            case "submit":
                guard let response = message.response, ["allow", "deny"].contains(response.decision) else { throw Failure("invalid decision") }
                let signature = try P256.Signing.ECDSASignature(derRepresentation: bytes(response.signature))
                guard publicKey.isValidSignature(signature, for: signedBytes(response)) else { throw Failure("signature rejected") }
                lock.lock()
                guard let entry = pending[response.payload], entry.decision == nil,
                      Date().timeIntervalSince1970 < entry.expires else {
                    lock.unlock(); throw Failure("unknown, expired or already decided challenge")
                }
                entry.decision = response.decision; lock.unlock()
                try sendLine(fd, Data("ACCEPTED".utf8))
            default: throw Failure("unknown command")
            }
        } catch {
            note("REJECTED \(error)")
            try? sendLine(fd, Data("ERROR \(error)".utf8))
        }
    }
}
func enroll(_ path: String, software: Bool) throws {
    guard !FileManager.default.fileExists(atPath: path), !FileManager.default.fileExists(atPath: path + ".pub") else { throw Failure("key path exists") }
    let key: KeyFile; let publicKey: Data
    if software {
        let privateKey = P256.Signing.PrivateKey()
        key = KeyFile(kind: "software-demo", wrappedKey: privateKey.rawRepresentation.base64EncodedString())
        publicKey = privateKey.publicKey.x963Representation
        note("DEMO KEY: no hardware protection or human presence")
    } else {
        guard SecureEnclave.isAvailable else { throw Failure("Secure Enclave unavailable; no fallback") }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, [.privateKeyUsage, .userPresence], &error) else { throw error!.takeRetainedValue() }
        let privateKey = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        key = KeyFile(kind: "secure-enclave-user-presence", wrappedKey: privateKey.dataRepresentation.base64EncodedString())
        publicKey = privateKey.publicKey.x963Representation
    }
    let data = try encoded(key)
    guard FileManager.default.createFile(atPath: path, contents: data, attributes: [.posixPermissions: 0o600]) else { throw Failure("cannot write key") }
    try publicKey.write(to: URL(fileURLWithPath: path + ".pub"))
    note("Enrolled \(key.kind); public key: \(path).pub")
}
func sign(_ path: String, payload: String, answer: String, software: Bool) throws -> Decision {
    guard ["allow", "deny"].contains(answer) else { throw Failure("decision must be allow or deny") }
    let challenge = try decoded(Challenge.self, bytes(payload))
    guard challenge.protocolName == protocolName, challenge.right == right,
          Date().timeIntervalSince1970 < challenge.expires else { throw Failure("wrong protocol/right or expired challenge") }
    note("DECISION \(answer): \(String(data: try encoded(challenge), encoding: .utf8)!)")
    let key = try decoded(KeyFile.self, Data(contentsOf: URL(fileURLWithPath: path)))
    var response = Decision(payload: payload, decision: answer, signature: "")
    let signature: P256.Signing.ECDSASignature
    if software {
        guard key.kind == "software-demo" else { throw Failure("not a demo key") }
        signature = try P256.Signing.PrivateKey(rawRepresentation: bytes(key.wrappedKey)).signature(for: signedBytes(response))
    } else {
        guard key.kind == "secure-enclave-user-presence" else { throw Failure("hardware-protected key required; no fallback") }
        let context = LAContext()
        context.localizedReason = "\(answer.capitalized) prototype authorization on \(challenge.host)"
        let privateKey = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: bytes(key.wrappedKey), authenticationContext: context)
        signature = try privateKey.signature(for: signedBytes(response))
    }
    response = Decision(payload: payload, decision: answer, signature: signature.derRepresentation.base64EncodedString())
    return response
}
func requestRight() throws {
    var auth: AuthorizationRef?
    guard AuthorizationCreate(nil, nil, [], &auth) == errAuthorizationSuccess, let auth else { throw Failure("AuthorizationCreate failed") }
    defer { AuthorizationFree(auth, []) }
    let status = right.withCString { name -> OSStatus in
        var item = AuthorizationItem(name: name, valueLength: 0, value: nil, flags: 0)
        return withUnsafeMutablePointer(to: &item) { pointer in
            var rights = AuthorizationRights(count: 1, items: pointer)
            return AuthorizationCopyRights(auth, &rights, nil, [.extendRights, .interactionAllowed], nil)
        }
    }
    print("AuthorizationCopyRights(\(right)) = \(status)")
    guard status == errAuthorizationSuccess else { throw Failure("right was not granted") }
}
let args = Array(CommandLine.arguments.dropFirst())
do {
    guard let command = args.first else { throw Failure("commands: enroll KEY | enroll-demo KEY | serve SOCKET PUBKEY [--automatic] | serve-demo SOCKET PUBKEY [--automatic] | pending SOCKET | sign KEY PAYLOAD allow|deny | sign-demo KEY PAYLOAD allow|deny | submit SOCKET RESPONSE.json | request | request-demo SOCKET") }
    switch command {
    case "enroll", "enroll-demo":
        guard args.count == 2 else { throw Failure("enroll KEY") }; try enroll(args[1], software: command == "enroll-demo")
    case "serve", "serve-demo":
        guard args.count == 3 || (args.count == 4 && args[3] == "--automatic") else { throw Failure("serve SOCKET PUBKEY [--automatic]") }
        let broker = try Broker(key: Data(contentsOf: URL(fileURLWithPath: args[2])), automatic: args.count == 4, demo: command == "serve-demo")
        let fd = try makeSocket(args[1], server: true)
        note("LISTENING \(args[1]) mode=\(args.count == 4 ? "standing-policy" : "human") demo=\(command == "serve-demo")")
        while true {
            let client = accept(fd, nil, nil)
            if client >= 0 { DispatchQueue.global().async { broker.handle(client) } }
        }
    case "pending":
        guard args.count == 2 else { throw Failure("pending SOCKET") }
        FileHandle.standardOutput.write(try exchange(Message(command: "pending"), args[1]) + Data([10]))
    case "sign", "sign-demo":
        guard args.count == 4 else { throw Failure("sign KEY PAYLOAD allow|deny") }
        try publish(sign(args[1], payload: args[2], answer: args[3], software: command == "sign-demo"))
    case "submit":
        guard args.count == 3 else { throw Failure("submit SOCKET RESPONSE.json") }
        let response = try decoded(Decision.self, Data(contentsOf: URL(fileURLWithPath: args[2])))
        let result = try exchange(Message(command: "submit", response: response), args[1])
        print(String(decoding: result, as: UTF8.self))
        guard result == Data("ACCEPTED".utf8) else { throw Failure("decision rejected") }
    case "request": try requestRight()
    case "request-demo":
        guard args.count == 2 else { throw Failure("request-demo SOCKET") }
        let result = try exchange(Message(command: "request"), args[1])
        print(String(decoding: result, as: UTF8.self))
        guard result == Data("ALLOW".utf8) else { throw Failure("request denied") }
    default: throw Failure("unknown command")
    }
} catch { note("ERROR: \(error)"); exit(1) }
