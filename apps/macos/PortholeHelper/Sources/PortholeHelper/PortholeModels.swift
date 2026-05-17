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
