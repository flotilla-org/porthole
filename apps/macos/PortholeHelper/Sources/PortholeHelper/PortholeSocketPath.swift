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
