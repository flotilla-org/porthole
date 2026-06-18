import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?
    private var statusMenuItem: NSMenuItem?
    private var loginItemMenuItem: NSMenuItem?
    private var supervisor: DaemonSupervisor?
    private var onboardingWindowController: OnboardingWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installStatusItem()

        let paths = BundlePaths.current()
        HelperStartup(
            migrateLaunchAgent: { LaunchAgentMigrator().migrate() },
            registerLoginItem: { LoginItemRegistrar().registerIfNeeded() },
            startSupervisor: { [weak self] in
                guard let self else { return }
                let nextSupervisor = DaemonSupervisor(cliURL: paths.cliURL) { [weak self] state in
                    self?.render(state)
                }
                supervisor = nextSupervisor
                nextSupervisor.start()
            },
            reportMigration: { [weak self] result in self?.reportLaunchAgentMigration(result) },
            reportLoginItem: { [weak self] result in self?.reportLoginItemRegistration(result) }
        ).start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        supervisor?.stopForQuit()
    }

    private func installStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.image = NSImage(systemSymbolName: "circle.grid.cross", accessibilityDescription: "Porthole")
            button.image?.isTemplate = true
        }

        let menu = NSMenu()
        let status = NSMenuItem(title: "Daemon: starting", action: nil, keyEquivalent: "")
        status.isEnabled = false
        statusMenuItem = status
        menu.addItem(status)
        let loginItem = NSMenuItem(title: "Login Item: checking", action: nil, keyEquivalent: "")
        loginItem.isEnabled = false
        loginItemMenuItem = loginItem
        menu.addItem(loginItem)
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Open Onboarding...", action: #selector(openOnboarding), keyEquivalent: "o"))
        menu.addItem(NSMenuItem(title: "Restart Daemon", action: #selector(restartDaemon), keyEquivalent: "r"))
        menu.addItem(NSMenuItem(title: "Quit Porthole", action: #selector(quit), keyEquivalent: "q"))
        item.menu = menu
        statusItem = item
    }

    private func render(_ state: DaemonSupervisor.State) {
        switch state {
        case .registering:
            statusMenuItem?.title = "Daemon: starting"
        case .running:
            statusMenuItem?.title = "Daemon: running"
        case .unresponsive:
            statusMenuItem?.title = "Daemon: not responding"
        case .needsApproval:
            statusMenuItem?.title = "Daemon: approve in System Settings"
        case .failed(let reason):
            statusMenuItem?.title = "Daemon: failed (\(reason))"
        }
    }

    private func reportLaunchAgentMigration(_ result: LaunchAgentMigrator.MigrationResult) {
        switch result {
        case .notNeeded:
            break
        case .migrated(let url):
            NSLog("retired legacy Porthole LaunchAgent at \(url.path(percentEncoded: false))")
        case .failed(let url, let reason):
            NSLog("failed to retire legacy Porthole LaunchAgent at \(url.path(percentEncoded: false)): \(reason)")
            statusMenuItem?.title = "LaunchAgent migration failed; daemon starting"
        }
    }

    private func reportLoginItemRegistration(_ result: LoginItemRegistrar.RegistrationResult) {
        switch result {
        case .alreadyEnabled:
            loginItemMenuItem?.title = "Login Item: enabled"
            NSLog("Porthole login item already enabled")
        case .registered:
            loginItemMenuItem?.title = "Login Item: registered"
            NSLog("registered Porthole login item")
        case .requiresApproval:
            loginItemMenuItem?.title = "Login Item: needs approval"
            NSLog("Porthole login item requires approval in System Settings")
        case .failed(let reason):
            loginItemMenuItem?.title = "Login Item: registration failed"
            NSLog("failed to register Porthole login item: \(reason)")
        }
    }

    @objc private func restartDaemon() {
        supervisor?.restart()
    }

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

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}
