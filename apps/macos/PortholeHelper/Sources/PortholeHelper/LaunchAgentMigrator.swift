import Darwin
import Foundation

struct LaunchAgentMigrator {
    enum MigrationResult: Equatable {
        case notNeeded
        case migrated(URL)
        case failed(URL, String)
    }

    struct Dependencies: Sendable {
        var homeDirectory: @Sendable () -> URL
        var fileExists: @Sendable (URL) -> Bool
        var bootout: @Sendable (URL) throws -> Void
        var removeFile: @Sendable (URL) throws -> Void

        static let live = Dependencies(
            homeDirectory: { FileManager.default.homeDirectoryForCurrentUser },
            fileExists: { FileManager.default.fileExists(atPath: pathString($0)) },
            bootout: LaunchAgentMigrator.bootout,
            removeFile: { try FileManager.default.removeItem(at: $0) }
        )
    }

    static let launchAgentLabel = "work.flotilla.porthole"
    static let plistName = "work.flotilla.porthole.plist"

    private let dependencies: Dependencies

    init(dependencies: Dependencies = .live) {
        self.dependencies = dependencies
    }

    func migrate() -> MigrationResult {
        let plistURL = dependencies
            .homeDirectory()
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("LaunchAgents", isDirectory: true)
            .appendingPathComponent(Self.plistName)

        guard dependencies.fileExists(plistURL) else {
            return .notNeeded
        }

        do {
            try dependencies.bootout(plistURL)
            try dependencies.removeFile(plistURL)
            return .migrated(plistURL)
        } catch {
            return .failed(plistURL, String(describing: error))
        }
    }

    private static func bootout(plistURL: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["bootout", "gui/\(getuid())", pathString(plistURL)]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice

        try process.run()
        process.waitUntilExit()
        if process.terminationStatus != 0 {
            NSLog("launchctl bootout returned \(process.terminationStatus) while retiring \(pathString(plistURL)); removing stale plist anyway")
        }
    }
}

private func pathString(_ url: URL) -> String {
    url.path(percentEncoded: false)
}

struct HelperStartup {
    var migrateLaunchAgent: () -> LaunchAgentMigrator.MigrationResult
    var registerLoginItem: () -> LoginItemRegistrar.RegistrationResult
    var startSupervisor: () -> Void
    var reportMigration: (LaunchAgentMigrator.MigrationResult) -> Void
    var reportLoginItem: (LoginItemRegistrar.RegistrationResult) -> Void

    func start() {
        let migrationResult = migrateLaunchAgent()
        reportMigration(migrationResult)
        let loginItemResult = registerLoginItem()
        reportLoginItem(loginItemResult)
        startSupervisor()
    }
}
