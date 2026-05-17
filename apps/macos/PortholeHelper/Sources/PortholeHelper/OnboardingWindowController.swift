import AppKit

@MainActor
final class OnboardingWindowController: NSWindowController {
    private let client: PortholeClientProtocol
    private let supervisor: DaemonSupervisor
    private var flow = OnboardingFlow()
    private var latestInfo: InfoResponse?
    private var refreshTask: Task<Void, Never>?

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
        window.contentMinSize = NSSize(width: 480, height: 360)
        super.init(window: window)
        installContent()
        render()
        refreshTask = Task { await refresh() }
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    deinit {
        refreshTask?.cancel()
    }

    private func installContent() {
        guard let window else { return }

        let contentView = NSView()
        let root = NSStackView()
        root.orientation = .vertical
        root.alignment = .leading
        root.spacing = 14
        root.translatesAutoresizingMaskIntoConstraints = false

        let titleLabel = NSTextField(labelWithString: "Porthole Onboarding")
        titleLabel.font = .preferredFont(forTextStyle: .title2)
        titleLabel.setAccessibilityLabel("Porthole Onboarding")

        statusLabel.font = .preferredFont(forTextStyle: .headline)
        statusLabel.lineBreakMode = .byWordWrapping
        statusLabel.setAccessibilityLabel("Onboarding status")

        permissionsStack.orientation = .vertical
        permissionsStack.alignment = .leading
        permissionsStack.spacing = 6
        permissionsStack.setAccessibilityLabel("System permissions")

        detailLabel.lineBreakMode = .byWordWrapping
        detailLabel.maximumNumberOfLines = 0
        detailLabel.setAccessibilityLabel("Onboarding detail")

        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .vertical)

        let buttonStack = NSStackView()
        buttonStack.orientation = .horizontal
        buttonStack.alignment = .centerY
        buttonStack.spacing = 8

        primaryButton.target = self
        primaryButton.action = #selector(primaryAction)
        primaryButton.bezelStyle = .rounded
        primaryButton.setAccessibilityLabel("Primary onboarding action")

        settingsButton.target = self
        settingsButton.action = #selector(openSettings)
        settingsButton.bezelStyle = .rounded
        settingsButton.setAccessibilityLabel("Open System Settings")

        refreshButton.target = self
        refreshButton.action = #selector(refreshAction)
        refreshButton.bezelStyle = .rounded
        refreshButton.setAccessibilityLabel("Refresh permission status")

        restartButton.target = self
        restartButton.action = #selector(restartAction)
        restartButton.bezelStyle = .rounded
        restartButton.setAccessibilityLabel("Restart daemon")

        buttonStack.addArrangedSubview(primaryButton)
        buttonStack.addArrangedSubview(settingsButton)
        buttonStack.addArrangedSubview(restartButton)
        buttonStack.addArrangedSubview(refreshButton)

        root.addArrangedSubview(titleLabel)
        root.addArrangedSubview(statusLabel)
        root.addArrangedSubview(permissionsStack)
        root.addArrangedSubview(detailLabel)
        root.addArrangedSubview(spacer)
        root.addArrangedSubview(buttonStack)

        contentView.addSubview(root)
        window.contentView = contentView

        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 24),
            root.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -24),
            root.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 24),
            root.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -24),
            permissionsStack.widthAnchor.constraint(equalTo: root.widthAnchor),
            detailLabel.widthAnchor.constraint(equalTo: root.widthAnchor),
            spacer.heightAnchor.constraint(greaterThanOrEqualToConstant: 1),
        ])
    }

    private func refresh() async {
        flow.apply(.loadStarted)
        render()
        do {
            let info = try await client.info()
            latestInfo = info
            flow.apply(.loadedInfo(info))
        } catch {
            flow.apply(.loadFailed(error.localizedDescription))
        }
        render()
    }

    private func requestActivePermission() async {
        guard let permission = flow.state.activePermissionName else { return }
        flow.apply(.requestStarted(permission))
        render()

        do {
            let outcome = try await client.requestPermissionPrompt(name: permission)
            flow.apply(.requestSucceeded(outcome))
        } catch {
            flow.apply(.requestFailed(error.localizedDescription))
        }
        render()
    }

    private func confirmGranted(permission: String, outcome: SystemPermissionPromptOutcome) async {
        flow.apply(.userConfirmedGrant(permission, requiresRestart: outcome.requiresDaemonRestart))
        render()

        if outcome.requiresDaemonRestart {
            await restartDaemonForOnboarding(permission: permission)
        } else {
            await waitForInfo(timeoutSeconds: 10)
        }
    }

    private func restartDaemonForOnboarding(permission: String) async {
        guard supervisor.restartCapability == .helperOwned else {
            flow.apply(.verificationFailed("An external daemon is running. Restart it manually, then refresh onboarding."))
            render()
            return
        }

        flow.apply(.userConfirmedGrant(permission, requiresRestart: true))
        render()
        supervisor.restart()
        await waitForInfo(timeoutSeconds: 10)
    }

    private func waitForInfo(timeoutSeconds: TimeInterval) async {
        let deadline = Date().addingTimeInterval(timeoutSeconds)
        while Date() < deadline {
            do {
                let info = try await client.info()
                latestInfo = info
                flow.apply(.verificationSucceeded(info))
                render()
                return
            } catch {
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
        }

        let permission = flow.state.activePermissionName ?? "permission"
        flow.apply(.verificationFailed("Daemon did not report updated state while verifying \(SettingsLinks.displayName(for: permission))."))
        render()
    }

    private func render() {
        let activePermission = flow.state.activePermissionName
        renderPermissions()

        settingsButton.isEnabled = activePermission.flatMap { SettingsLinks.link(for: $0) } != nil
        refreshButton.isEnabled = true
        restartButton.isEnabled = supervisor.restartCapability == .helperOwned

        switch flow.state {
        case .idle, .loadingInfo:
            statusLabel.stringValue = "Checking daemon..."
            detailLabel.stringValue = "Reading current permission state from portholed."
            primaryButton.title = "Request Permission"
            primaryButton.isEnabled = false
            settingsButton.isEnabled = false
            restartButton.isEnabled = false
        case .ready(_, let activePermission):
            if let activePermission {
                let name = SettingsLinks.displayName(for: activePermission.name)
                statusLabel.stringValue = "\(name) permission needed"
                detailLabel.stringValue = activePermission.purpose
                primaryButton.title = "Request Permission"
                primaryButton.isEnabled = true
            } else {
                statusLabel.stringValue = "All permissions granted"
                detailLabel.stringValue = "No missing system permissions are advertised by the daemon."
                primaryButton.title = "Complete"
                primaryButton.isEnabled = false
            }
        case .requesting(let permission):
            statusLabel.stringValue = "Requesting \(SettingsLinks.displayName(for: permission))"
            detailLabel.stringValue = "macOS may show a system permission prompt."
            primaryButton.title = "Requesting..."
            primaryButton.isEnabled = false
            settingsButton.isEnabled = false
            restartButton.isEnabled = false
        case .waitingForUser(let permission, let outcome):
            statusLabel.stringValue = "Waiting for \(SettingsLinks.displayName(for: permission))"
            detailLabel.stringValue = waitingDetail(for: outcome)
            primaryButton.title = "Check Again"
            primaryButton.isEnabled = true
        case .restarting(let permission):
            statusLabel.stringValue = "Restarting daemon"
            detailLabel.stringValue = "Restarting portholed before verifying \(SettingsLinks.displayName(for: permission))."
            primaryButton.title = "Restarting..."
            primaryButton.isEnabled = false
            settingsButton.isEnabled = false
            restartButton.isEnabled = false
        case .verifying(let permission):
            statusLabel.stringValue = "Verifying \(SettingsLinks.displayName(for: permission))"
            detailLabel.stringValue = "Waiting for the daemon to report updated permission state."
            primaryButton.title = "Verifying..."
            primaryButton.isEnabled = false
            settingsButton.isEnabled = false
            restartButton.isEnabled = false
        case .complete:
            statusLabel.stringValue = "Onboarding complete"
            detailLabel.stringValue = "All advertised system permissions are granted."
            primaryButton.title = "Complete"
            primaryButton.isEnabled = false
            settingsButton.isEnabled = false
            restartButton.isEnabled = false
        case .blocked(let message):
            statusLabel.stringValue = "Action needed"
            detailLabel.stringValue = message
            primaryButton.title = "Refresh"
            primaryButton.isEnabled = true
            settingsButton.isEnabled = false
        }
    }

    private func renderPermissions() {
        for view in permissionsStack.arrangedSubviews {
            permissionsStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        let permissions = latestInfo?.adapters.flatMap(\.systemPermissions) ?? []
        guard !permissions.isEmpty else {
            let empty = NSTextField(labelWithString: "No system permissions reported yet.")
            empty.textColor = .secondaryLabelColor
            empty.setAccessibilityLabel("No system permissions reported yet")
            permissionsStack.addArrangedSubview(empty)
            return
        }

        for permission in permissions {
            let row = NSStackView()
            row.orientation = .horizontal
            row.alignment = .centerY
            row.spacing = 8
            row.widthAnchor.constraint(equalTo: permissionsStack.widthAnchor).isActive = true

            let name = NSTextField(labelWithString: SettingsLinks.displayName(for: permission.name))
            name.font = .preferredFont(forTextStyle: .body)
            name.setContentHuggingPriority(.defaultLow, for: .horizontal)

            let state = NSTextField(labelWithString: permission.granted ? "Granted" : "Needed")
            state.textColor = permission.granted ? .systemGreen : .systemOrange
            state.alignment = .right
            state.setContentHuggingPriority(.required, for: .horizontal)

            row.addArrangedSubview(name)
            row.addArrangedSubview(state)
            row.setAccessibilityLabel("\(SettingsLinks.displayName(for: permission.name)): \(state.stringValue)")
            permissionsStack.addArrangedSubview(row)
        }
    }

    private func waitingDetail(for outcome: SystemPermissionPromptOutcome) -> String {
        let permission = SettingsLinks.displayName(for: outcome.permission)
        let restart = outcome.requiresDaemonRestart ? " The daemon will restart before verification." : ""
        if outcome.notes.isEmpty {
            return "Grant \(permission) in System Settings, then check again.\(restart)"
        }
        return "\(outcome.notes) Grant \(permission) in System Settings, then check again.\(restart)"
    }

    @objc private func primaryAction() {
        switch flow.state {
        case .ready:
            refreshTask = Task { await requestActivePermission() }
        case .waitingForUser(let permission, let outcome):
            refreshTask = Task { await confirmGranted(permission: permission, outcome: outcome) }
        case .blocked:
            refreshTask = Task { await refresh() }
        default:
            break
        }
    }

    @objc private func openSettings() {
        guard let permission = flow.state.activePermissionName,
              let url = SettingsLinks.link(for: permission)
        else { return }
        NSWorkspace.shared.open(url)
    }

    @objc private func refreshAction() {
        refreshTask = Task { await refresh() }
    }

    @objc private func restartAction() {
        guard supervisor.restartCapability == .helperOwned else { return }
        if let permission = flow.state.activePermissionName {
            refreshTask = Task { await restartDaemonForOnboarding(permission: permission) }
        } else {
            supervisor.restart()
        }
    }
}
