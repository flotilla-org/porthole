import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?
    private var statusMenuItem: NSMenuItem?
    private var supervisor: DaemonSupervisor?
    private var onboardingWindowController: OnboardingWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installStatusItem()

        let paths = BundlePaths.current()
        supervisor = DaemonSupervisor(daemonURL: paths.daemonURL, cliURL: paths.cliURL) { [weak self] state in
            self?.render(state)
        }
        supervisor?.start()
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
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Open Onboarding...", action: #selector(openOnboarding), keyEquivalent: "o"))
        menu.addItem(NSMenuItem(title: "Restart Daemon", action: #selector(restartDaemon), keyEquivalent: "r"))
        menu.addItem(NSMenuItem(title: "Quit Porthole", action: #selector(quit), keyEquivalent: "q"))
        item.menu = menu
        statusItem = item
    }

    private func render(_ state: DaemonSupervisor.State) {
        switch state {
        case .stopped:
            statusMenuItem?.title = "Daemon: stopped"
        case .running(let pid):
            statusMenuItem?.title = "Daemon: running (\(pid))"
        case .runningExternal:
            statusMenuItem?.title = "Daemon: already running"
        case .crashed(let status):
            statusMenuItem?.title = "Daemon: restarting after exit \(status)"
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
