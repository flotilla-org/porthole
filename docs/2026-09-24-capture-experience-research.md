# Capture experience research: agent desktops, placement and audio

Status: research only, no decisions. Requested by the operator on 2026-09-24
as longer-term direction work. It feeds the roadmap's "future macOS display
controlled by Porthole" note and the Linux purpose-built compositor note; it
does not change the active milestones.

Three threads were researched in parallel against primary sources (Apple and
Microsoft documentation, SDK headers, WWDC transcripts, upstream compositor,
PipeWire and streaming-stack source). Each claim below carries its source.
Anything not confirmed by a primary source is marked **UNVERIFIED**. Nothing was
run or tested; every "experiment" below is proposed, not performed.

## Starting point in Porthole today

- macOS captures no audio: the ScreenCaptureKit shim sets `capturesAudio = NO`
  and every stream uses a per-window `desktopIndependentWindow` filter
  (`crates/porthole-adapter-macos/src/sck_capture_shim.m`).
- macOS launch goes through `NSWorkspace.OpenConfiguration` with `activates`
  left at its default of true, so a launched app takes the foreground. Placement
  is an Accessibility `AXPosition`/`AXSize` write after the window exists
  (`launch.rs`, `placement.rs`).
- KWin capture goes through the portal with a persisted grant; Windows uses
  `PrintWindow` plus `SetForegroundWindow` and `SendInput`.

## Synthesis

**Linux has the clean model.** An agent-owned headless compositor makes Porthole
the compositor's trusted client: capture through Wayland protocols or a PipeWire
node the compositor publishes, input through virtual-keyboard/pointer or EIS,
no portal dialog, and the human's seat, focus and cursor are never touched.
Gamescope (headless backend, PipeWire `Video/Source` node, own EIS socket) and a
nested `kwin_wayland --virtual` (reuses the existing screencast consumer, minus
the portal) are the closest to working. Per-app audio is a null sink per agent
plus `PIPEWIRE_NODE`/`PULSE_SINK` in the launched process's environment; the
human hears nothing. KWin master gates its screencast, fake-input and
window-management protocols only for sandboxed clients, so an unsandboxed
Porthole could bind them directly even in the human's session (source-level,
release boundary unverified).

**Windows solves capture, not isolation.** Apollo's SudoVDA virtual monitor is an
IddCx display on the human's one desktop: captured like any monitor, but the
cursor and window focus can still reach it. Per-process audio capture exists
(process loopback, Windows 10 build 20348 and later) but nothing documented stops
the human hearing the app; Sunshine silences the host by switching the global
default device. `SendInput` goes to the single input desktop, and a hidden
`CreateDesktop` desktop receives no input stream.

**macOS is the hard case, and the two problems split cleanly.**

- *Placement.* Three low-risk changes fall out directly: launch with
  `activates = false` so the app stops taking the foreground, pass
  `-ApplePersistenceIgnoreState YES` so saved window state is ignored, and move
  the window on `kAXWindowCreatedNotification`. No public API moves a window
  before its first frame, so a flash at the original position is likely; its
  length is unmeasured. Private SkyLight moves need SIP changes and are out.
- *Virtual display.* The private `CGVirtualDisplay` API is usable (DeskPad,
  BetterDisplay and Chromium's test harness use it; no entitlement outside the
  sandbox; macOS 13 through 26 on Apple Silicon). It gives Porthole a display
  with known resolution and scale, and per-window capture keeps working there.
  It does **not** isolate the human: `CGConfigureDisplayOrigin` forbids gaps so
  the cursor can always reach it, there is one keyboard focus per session, and
  since macOS 14 activation is a request that either steals the human's focus
  or silently fails. Accessibility value-setting and actions are the only
  focus-free input path, and that is app-dependent.
- *Audio.* ScreenCaptureKit audio is scoped to the application by design, never
  to a window, and it never mutes the source. Core Audio process taps
  (macOS 14.2 and later) are the fit: tap the launched process tree, use
  `CATapMuteBehavior.muted` or `.mutedWhenTapped` so the human does not hear
  it, edit the tapped set live, and need only the audio-only TCC grant. Taps
  address HAL process objects, not PIDs, so Porthole must watch the process
  object list and cover helper processes (Chromium plays audio from a separate
  process). Virtual audio devices do not solve per-app routing because macOS
  has no per-process default output; AudioUnits/AUHAL are irrelevant to
  capturing another app. macOS 26 adds tapping by bundle ID, which would also
  catch the human's own copy of the same app.
- *True isolation* on macOS means a Virtualization.framework guest per agent. A
  second user session gets no keyboard or mouse input while switched out.

## Proposed experiments, in order of cost

1. **Quiet launch and Accessibility placement (macOS, low).** `activates =
   false`, `-ApplePersistenceIgnoreState YES`, AX move on window creation, and a
   "park in a corner" placement default. Measure foreground changes and frames
   shown at the original position across TextEdit, Safari and an Electron app.
2. **Process tap prototype (macOS 14.2+, low).** Tap a launched process tree
   with `.muted`; confirm silence at the speakers, non-zero captured samples,
   system alerts unaffected, and live PID additions without a restart. Check
   the TCC prompt from the bundled daemon identity.
3. **`CGVirtualDisplay` probe (macOS, medium).** A small ObjC tool from
   DeskPad's header on macOS 14, 15 and 26: presence in
   `SCShareableContent.displays`, capture, lock/sleep/restart behaviour,
   Spaces and menu bar, and whether an AX-only drive changes the human's
   frontmost app.
4. **Linux agent-desktop spike (medium).** Nested `kwin_wayland --virtual` on
   its own D-Bus session with direct `zkde_screencast` and `connectToEIS`, or
   gamescope headless feeding Jackstay's PipeWire consumer, plus a per-launch
   null sink.
5. **Windows process loopback and WGC (low).** Process-tree loopback keyed on
   the launched PID, and `CreateForWindow` capture with the border off as a
   replacement for `PrintWindow`.

Input policy is a cross-cutting decision regardless of platform: default to
Accessibility (or the compositor-local equivalent) and treat "take the
keyboard" as an explicit operation, because on macOS 14 and later focus
acquisition can fail silently.

The three research threads follow in full.

## Thread 1: macOS window placement and virtual displays

Research date: 2026-09-24. Sources are primary: Apple docs (fetched as JSON from `developer.apple.com/tutorials/data/documentation/...`), WWDC transcripts, Apple Developer Forums, Apple release notes, the `apple/device-management` schema, and open-source code (DeskPad, Chromium). Anything without a primary source is marked **UNVERIFIED**.

Porthole context (read from the checkout, not changed): launch goes through `NSWorkspaceOpenConfiguration` with `createsNewApplicationInstance=true`, plus arguments and environment (`crates/porthole-adapter-macos/src/launch.rs`, `launch_configuration`). It leaves `activates` at its default of `true`. `place_surface` writes `AXPosition` and `AXSize` (`placement.rs`). `displays()` enumerates `CGDisplay::active_displays()` (`display.rs`). `focus()` calls `NSRunningApplication.activateWithOptions` and then `AXRaise` (`close_focus.rs`). Keys go through `CGEventPost` at the HID tap. Text goes through `CGEventPostToPid`. Button-down calls `focus()` first (`input.rs`). Capture uses `SCContentFilter initWithDesktopIndependentWindow:` (`sck_capture_shim.m`).

### Summary

- **Feasible, low risk:** stop the launch from taking focus by setting `NSWorkspace.OpenConfiguration.activates = false` ([doc](https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/activates)). Neutralise saved window state with `-ApplePersistenceIgnoreState YES` ([AppKit release notes 10.7](https://developer.apple.com/library/archive/releasenotes/AppKit/RN-AppKitOlderNotes/index.html)) and pin frame autosave keys with `-"NSWindow Frame <name>" "<rect>"` in the argument domain ([UserDefaults.argumentDomain](https://developer.apple.com/documentation/foundation/userdefaults/argumentdomain)). Then move the window with AX. The window is still drawn at its original position first. No public API moves a window before its first frame.
- **Feasible, medium risk:** a private `CGVirtualDisplay` gives Porthole its own display. It is used by DeskPad, BetterDisplay and Chromium's test harness. It needs no entitlement outside the sandbox, and it works on macOS 13 through 26 including Apple Silicon. It is **not** isolated from the human, though. It sits in the shared display arrangement, and `CGConfigureDisplayOrigin` forbids gaps ([doc](https://developer.apple.com/documentation/coregraphics/cgconfiguredisplayorigin(_:_:_:_:))), so the cursor can reach it. There is also one keyboard focus per session, so key events posted at the HID tap still follow the active app.
- **Not feasible without focus contention:** typing into a background app. `CGEventPostToPid` is reported not to reach a background app's dialog ([forum 724835](https://developer.apple.com/forums/thread/724835), no Apple reply). Activation is a request since macOS 14 ([WWDC23 10054](https://developer.apple.com/videos/play/wwdc2023/10054/)). Only AX value setting and AX actions avoid focus changes.
- **True isolation** means a Virtualization.framework macOS VM. A second user session does not work: switched-out sessions get no keyboard or mouse input ([Fast User Switching guide](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPMultipleUsers/Concepts/FastUserSwitching.html)).

### A. Initial window placement

**What decides the first position** (inside the app, so outside Porthole's control):
- Frame autosave. `NSWindow.setFrameAutosaveName(_:)` stores the frame in defaults ([doc](https://developer.apple.com/documentation/appkit/nswindow/setframeautosavename(_:))) and `setFrameUsingName(_:)` reads it back ([doc](https://developer.apple.com/documentation/appkit/nswindow/setframeusingname(_:))). The key has the form `"NSWindow Frame <name>"` (see the `defaults delete NSGlobalDomain "NSWindow Frame NXSavePanel"` example in the [AppKit older release notes](https://developer.apple.com/library/archive/releasenotes/AppKit/RN-AppKitOlderNotes/index.html)).
- Cascading. `cascadeTopLeft(from:)` offsets each new window from the previous one ([doc](https://developer.apple.com/documentation/appkit/nswindow/cascadetopleft(from:))).
- `center()` places the window "exactly in the center horizontally and somewhat above center vertically" ([doc](https://developer.apple.com/documentation/appkit/nswindow/center())).
- State restoration. `isRestorable` defaults to true for titled windows, and "the system tries to recreate the window and restore its configuration" on the next launch ([doc](https://developer.apple.com/documentation/appkit/nswindow/isrestorable)).
- Which screen the window lands on (for example, the screen with the key window or the menu bar): **UNVERIFIED**. No Apple doc found.
- The Dock's per-app "Desktop on Display N" assignment makes an app open "in the current space on a specific display" ([Apple Support: Spaces](https://support.apple.com/guide/mac-help/work-in-multiple-spaces-mh14112/mac)). This is a user setting. A programmatic equivalent is **UNVERIFIED**.

**What a launcher can influence:**
- Command-line defaults. Arguments such as `-Key value` go into the volatile argument domain, which "override[s] most other domains" ([UserDefaults.argumentDomain](https://developer.apple.com/documentation/foundation/userdefaults/argumentdomain)). Porthole already passes `arguments` through `NSWorkspace.OpenConfiguration`. Two useful keys:
  - `-ApplePersistenceIgnoreState YES`: "existing restorable state and Untitled documents are ignored ... intended for automated tests that want to start with a clean environment" ([AppKit release notes, "Ignoring Existing Restorable State"](https://developer.apple.com/library/archive/releasenotes/AppKit/RN-AppKitOlderNotes/index.html)).
  - `-"NSWindow Frame <autosaveName>" "x y w h sx sy sw sh"`: seeds a frame autosave for a known window name. The string format is **UNVERIFIED**. Read an existing value with `defaults read` to learn it.
  - `NSQuitAlwaysKeepsWindows` (the "Close windows when quitting" preference) is widely used, but I found no Apple doc for it: **UNVERIFIED**.
- `defaults write <bundle-id> ...` persists the same keys. It changes the human's own preferences for that app, so it should be avoided.
- `NSWorkspace.OpenConfiguration`:
  - `activates` defaults to `true`, "which causes the system to bring the app to the foreground" ([doc](https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/activates)).
  - `hides` makes "the app ... hide itself after it launches" ([doc](https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/hides)).
  - `addsToRecentItems` ([doc](https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/addstorecentitems)).
  - No option sets the target display or a frame.
- Environment variables have no AppKit placement effect that I could find (**UNVERIFIED**).

**What can be done after launch:**
- AX. `AXUIElementSetAttributeValue` on `kAXPositionAttribute` and `kAXSizeAttribute` ([doc](https://developer.apple.com/documentation/applicationservices/1460434-axuielementsetattributevalue)). This is what Porthole does today. Observe `kAXWindowCreatedNotification` through `AXObserver` to react as early as possible. `kAXWindowMovedNotification` fires "at the end of the window-move operation, not during it" ([doc](https://developer.apple.com/documentation/applicationservices/kaxwindowmovednotification)).
- `CGWindowList` is read-only and cannot move windows. `CGWindowListCreateImage` is listed as deprecated in the Core Graphics index.
- Private SkyLight (`SLSMoveWindow` / `CGSMoveWindow`, and moving windows between Spaces). Moving another process's windows needs yabai's Dock scripting addition. Its wiki lists "move/swap/create/destroy space", "control window layers" and "sticky windows" as needing SIP partially disabled ([yabai wiki](https://github.com/koekeishiya/yabai/wiki/Disabling-System-Integrity-Protection)). This is not acceptable for an operator's machine.
- **Before the first frame:** no documented guarantee. AX needs the window to exist, which means the app has already created it and usually ordered it front, so at least one frame at the original position is likely. The latency figures are **UNVERIFIED** and need measuring.

### B. Virtual displays (`CGVirtualDisplay`, private CoreGraphics)

**API shape**, from class dumps in two independent projects:
- DeskPad [`CGVirtualDisplayPrivate.h`](https://github.com/Stengo/DeskPad/blob/main/DeskPad/CGVirtualDisplayPrivate.h) (credited to Khaos Tian, 2021).
- Chromium [`ui/display/mac/test/virtual_display_util_mac.mm`](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/ui/display/mac/test/virtual_display_util_mac.mm) ("These interfaces were generated from CoreGraphics binaries").

The classes:
- `CGVirtualDisplayDescriptor`: `name`, `maxPixelsWide`, `maxPixelsHigh`, `sizeInMillimeters`, `vendorID`, `productID`, `serialNum`, `queue` / `setDispatchQueue:`, `terminationHandler`, and colour primaries (`redPrimary`, `whitePoint`, ...).
- `CGVirtualDisplay`: `-initWithDescriptor:`, `-applySettings:` (returns `BOOL`), and `displayID` (a `CGDirectDisplayID`).
- `CGVirtualDisplaySettings`: `modes`, `hiDPI` (unsigned int), and `rotation` (in the Chromium dump).
- `CGVirtualDisplayMode`: `-initWithWidth:height:refreshRate:`, plus an overload with `transferFunction:` marked `API_AVAILABLE(macos(13.3))` in Chromium.

The display exists for as long as the `CGVirtualDisplay` object is alive. Chromium keeps displays in a global map and removes one by erasing its entry. None of these classes appear in Apple's public Core Graphics documentation (the JSON endpoint for `coregraphics/cgvirtualdisplay` returns nothing).

**Entitlements and SIP:** DeskPad is sandboxed and needs only `com.apple.security.temporary-exception.mach-lookup.global-name = com.apple.VirtualDisplay` ([DeskPad.entitlements](https://github.com/Stengo/DeskPad/blob/main/DeskPad/DeskPad.entitlements)). An unsandboxed daemon such as Porthole therefore needs no entitlement. That last step is an inference, **UNVERIFIED** by test. Nothing in either codebase touches SIP. The process needs a WindowServer (Aqua) session: Chromium's `IsAPIAvailable()` returns false when running headless.

**Versions and hardware:**
- DeskPad's deployment target is macOS 13.0 ([project.pbxproj](https://github.com/Stengo/DeskPad/blob/main/DeskPad.xcodeproj/project.pbxproj)).
- Chromium notes that "macOS 14 expects different virtual displays to have different serial numbers" and "macOS 14 expects a non-zero vendorID".
- The BetterDisplay README lists virtual screens on Apple Silicon and Intel for macOS 13.2 through 26 (BetterDisplay 4) ([README](https://github.com/waydabber/BetterDisplay)). BetterDisplay itself is closed source.

**Modes and HiDPI:**
- DeskPad sets `maxPixels` to 5120×2160, `hiDPI = 1` and a list of modes from 1280×720 up to 5120×2160 at 60 Hz ([ScreenViewController.swift](https://github.com/Stengo/DeskPad/blob/main/DeskPad/Frontend/Screen/ScreenViewController.swift)).
- Chromium, for HiDPI, sets the mode to half the pixel size (`width/2`) against the descriptor's full `maxPixels`.
- Chromium defines presets up to 6016×3384.
- The maximum number of virtual displays and any refresh-rate ceiling are **UNVERIFIED**. Chromium queries up to 32 online displays (`kMaxDisplaysToQuery = 32`), but that is a buffer size, not an OS limit.

**Known defects:**
- With multiple virtual displays and a reconnect, `SCStream` streams the wrong virtual framebuffer. An Apple DTS engineer called it "eminently bugworthy" (FB17797423) ([forum 786829](https://developer.apple.com/forums/thread/786829)).
- Chromium works around flaky timeouts on the first display removal by removing a second display at the same time.
- Chromium holds `kIOPMAssertionTypeNoDisplaySleep` while virtual displays exist. This suggests display sleep interferes with them. The exact failure is **UNVERIFIED**.

**Capture:** a virtual display is a normal `CGDirectDisplayID`. DeskPad mirrors its own display with `CGDisplayStream(dispatchQueueDisplay: display.displayID, ...)`. That it also appears in `SCShareableContent.displays` and works with `SCContentFilter(display:excludingWindows:)` is implied by the DTS-acknowledged forum report above (the user filters on a virtual `displayID`). Porthole's per-window filter is display-independent, so it works on any display: "a single window filter always includes full window content even when the source window is off-screen or occluded" ([WWDC22 10155](https://developer.apple.com/videos/play/wwdc2022/10155/)).

### C. Interference analysis for a virtual "agent desktop"

1. **Cursor.** `CGConfigureDisplayOrigin` places displays "as close as possible to the requested location, without overlapping or leaving a gap between displays" ([doc](https://developer.apple.com/documentation/coregraphics/cgconfiguredisplayorigin(_:_:_:_:))). So a virtual extended display is always reachable by the pointer.
   - Mirroring (`CGConfigureDisplayMirrorOfDisplay`, [doc](https://developer.apple.com/documentation/coregraphics/cgconfiguredisplaymirrorofdisplay(_:_:_:))) removes it from the extended desktop, but then it shows the human's content and is useless.
   - Whether a corner-only adjacency stops the cursor is **UNVERIFIED**.
   - Porthole's own mouse clicks move the one shared cursor. `CGEventPost` mouse events warp it. `CGWarpMouseCursorPosition` moves the cursor "without generating ... an event" ([doc](https://developer.apple.com/documentation/coregraphics/cgwarpmousecursorposition(_:))), but it is still the human's cursor.
2. **Keyboard.** Cocoa "dispatches most key events to the first responder of the key window" ([Event Architecture](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/EventArchitecture/EventArchitecture.html)). HID-tap keystrokes go to the active app, whichever display it is on.
   - `CGEventPostToPid` ([doc](https://developer.apple.com/documentation/coregraphics/cgevent/posttopid(_:))) targets a process, but a developer reports it does not drive a background app's dialog ([forum 724835](https://developer.apple.com/forums/thread/724835), no Apple reply). Porthole's own comments already note it follows the app's key window.
   - Focus-free alternatives are to set `kAXValueAttribute` on text fields and to use `AXPress` / `AXUIElementPerformAction` ([AXUIElement.h](https://developer.apple.com/documentation/applicationservices/axuielement_h)). That these never activate the target is **UNVERIFIED** in general and app-dependent.
3. **Activation.** Since macOS 14, "Activate is now a request, as opposed to a command". `activateIgnoringOtherApps` is ignored and deprecated. `yieldActivation(to:)` lets the *active* app hand over ([WWDC23 10054](https://developer.apple.com/videos/play/wwdc2023/10054/); [`activate(from:options:)`](https://developer.apple.com/documentation/appkit/nsrunningapplication/activate(from:options:)); [`yieldActivation(to:)`](https://developer.apple.com/documentation/appkit/nsapplication/yieldactivation(to:))). So Porthole's `focus()` will steal focus, or silently fail to take it, depending on context. It cannot focus a window on the virtual display without deactivating the human's app.
4. **Spaces.** With "Displays have separate Spaces" on, each display has its own Spaces ([Desktop & Dock settings](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac)). Whether a virtual display gets independent Spaces in the same way is **UNVERIFIED**, though it is expected since macOS treats it as a monitor. "When switching to an application, switch to a Space with open windows" (same page) can jump the human's view when an agent app activates.
5. **Menu bar, sleep, persistence.** A menu bar on every display when separate Spaces is on: **UNVERIFIED** (no Apple page found). Display sleep: see Chromium's `NoDisplaySleep` assertion. Persistence across lock and sleep: **UNVERIFIED**. The display lives only as long as the creating process holds the object, so a Porthole restart removes it and macOS moves its windows onto the remaining displays. DeskPad's README describes launching it as "equivalent to plugging in a monitor" ([README](https://github.com/Stengo/DeskPad)).

### D. Alternatives

- **Separate user session (Fast User Switching):** switched-out processes keep running and drawing but "do not receive input from the keyboard and mouse". To them, "the monitor would appear to be in sleep mode" ([Apple guide](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPMultipleUsers/Concepts/FastUserSwitching.html)). DTS advises that GUI capture belong in a per-session agent, not a daemon ([forum 814152](https://developer.apple.com/forums/thread/814152)). That ScreenCaptureKit returns frames for an inactive session: **UNVERIFIED**, likely not. Verdict: not viable for driving apps.
- **Virtualization.framework macOS VM:** `VZMacGraphicsDeviceConfiguration` gives the guest its own display. `VZVirtualMachineView` forwards keyboard and mouse to the VM only when the view has them ([docs](https://developer.apple.com/documentation/virtualization/vzvirtualmachineview); [sample](https://developer.apple.com/documentation/virtualization/running-macos-in-a-virtual-machine-on-apple-silicon)). Porthole would run inside the guest. This gives full isolation of focus, cursor and Spaces. The costs are heavy: a guest install, the apps installed inside it, and GPU limits. The macOS licence limit on concurrent VMs is **UNVERIFIED** here.
- **Hidden Space:** `onScreenWindowsOnly: true` returns only on-screen windows ([WWDC22 10156](https://developer.apple.com/videos/play/wwdc2022/10156/); [doc](https://developer.apple.com/documentation/screencapturekit/scshareablecontent/getexcludingdesktopwindows(_:onscreenwindowsonly:completionhandler:))). Porthole must enumerate with `false` to find windows on other Spaces. The desktop-independent filter captures windows that are "completely off-screen or moved to other displays" ([WWDC22 10155](https://developer.apple.com/videos/play/wwdc2022/10155/)). Whether that includes non-current Spaces is **UNVERIFIED**. Minimised windows pause the stream (same session). Creating or moving Spaces needs private APIs and SIP changes (see yabai). Input still needs focus, and focus switches Spaces.

### E. Recent Apple changes

- macOS 14:
  - Cooperative activation (above).
  - `SCContentSharingPicker` ([doc](https://developer.apple.com/documentation/screencapturekit/sccontentsharingpicker)), `SCScreenshotManager` ([doc](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager)), and `presenterOverlayPrivacyAlertSetting` ([doc](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/presenteroverlayprivacyalertsetting)).
- macOS 14.4:
  - `com.apple.developer.persistent-content-capture`, a VNC-only entitlement for persistent capture access that needs Apple approval ([doc](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.persistent-content-capture)).
  - `SCShareableContent.getCurrentProcessShareableContent` ([doc](https://developer.apple.com/documentation/screencapturekit/scshareablecontent/getcurrentprocessshareablecontent(completionhandler:))).
- macOS 15 re-prompts for Screen Recording periodically. The only Apple-documented control I found is the MDM restriction `forceBypassScreenCaptureAlert` (macOS 15.1+, supervised only, "the system bypasses the presentation of a screen capture alert") ([apple/device-management](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.applicationaccess.yaml)). The monthly cadence and the `~/Library/Group Containers/group.com.apple.replayd/ScreenCaptureApprovals.plist` file are known only from non-Apple sources: **UNVERIFIED**. I found no `NSApplication` defaults key that controls it: **UNVERIFIED**.
- No public virtual-display API has shipped as of the current Apple documentation (checked 2026-09-24). `CGVirtualDisplay` is absent from the Core Graphics docs, and the ScreenCaptureKit index has no display-creation API. How Sidecar and iPhone Mirroring create displays internally is **UNVERIFIED**, with no public API.

### Recommendation candidates

1. **Quiet launch plus clean state** (low risk). Set `activates = false` on `NSWorkspace.OpenConfiguration`. Inject `-ApplePersistenceIgnoreState YES` into the args (the operator can opt out). Place the window with AX as soon as `kAXWindowCreatedNotification` fires. *Experiment:* launch TextEdit, Safari and an Electron app 20 times each while another app is frontmost. Record whether the frontmost app changed, and the time from launch to `AXPosition` applied. Record SCK frames to count how many frames show the window at its original position.
2. **"Parking" placement on a real display** (low risk). Choose a corner of the display the human is least likely to use, from `displays()`, and add a `place` default. The window is still visible, and apps may clamp it with `constrainFrameRect:toScreen:`. *Experiment:* place partly off-screen and check whether AX accepts the position and whether SCK still captures the full content.
3. **`CGVirtualDisplay` agent display** (medium risk: private API, cursor reachable, keyboard focus shared). Create one display in the daemon's Aqua session with a unique `serialNum` and a non-zero `vendorID`, place it at the far edge of the arrangement, and launch plus AX-move apps onto it. Use AX actions and values for input, and fall back to focus plus HID keys only with an operator-visible notice. *Experiment:* a minimal Swift or ObjC tool based on DeskPad's header on macOS 14, 15 and 26 (Apple Silicon). Measure:
   - that it appears in `SCShareableContent.displays`;
   - capture of the display and of its windows;
   - behaviour across lock, sleep and wake and across a daemon restart;
   - Spaces and menu-bar behaviour;
   - whether the human's frontmost app changes during an AX-only drive.
4. **Per-agent macOS VM** (high cost, full isolation). Run Porthole inside a `VZVirtualMachine` guest. *Experiment:* use the Apple sample to run a guest, run Porthole in it, and measure boot time, capture FPS and input fidelity.
5. **Input policy change regardless of the choice above.** Default to AX value setting and AX actions. Treat `focus()` plus HID input as an explicit "take the keyboard" operation, since macOS 14 activation can fail silently.

### Open questions

- Does the cursor cross a corner-only adjacency to a virtual display? Can it be kept off a display without private APIs?
- Do virtual displays survive lock, sleep and wake, and do they show a menu bar and their own Spaces?
- Do `CGEventPostToPid` keystrokes reach a background app's key window in current macOS, for Cocoa text views as opposed to dialogs?
- Does the desktop-independent SCK filter keep delivering frames for a window on a non-current Space?
- How many virtual displays can exist, and at what resolution cap? What is the GPU and memory cost?
- Does Apple's macOS licence allow as many concurrent VMs as Porthole would need?

## Thread 2: macOS audio capture and isolation

Researched 2026-09-24 from primary sources: Apple documentation JSON, SDK headers (MacOSX15.5/26.5 SDK mirror at github.com/alexey-lysiuk/macos-sdk), WWDC transcripts, Apple sample code, and open-source code. Anything not confirmed is marked **UNVERIFIED**.

### Summary

- **What Porthole does today:** it captures no audio. Both the stream config and the screenshot config set `config.capturesAudio = NO`, and each stream uses a per-window filter, `-[SCContentFilter initWithDesktopIndependentWindow:]` (`crates/porthole-adapter-macos/src/sck_capture_shim.m` lines 166-179, 239, 431, 453).
- **Why audio looks "per-app (or window?)":** ScreenCaptureKit (SCK) filters audio **only at the application level**. A single-window filter captures **all audio from the app that owns that window**, including audio from its other windows. It never captures only the window's audio, and it never captures the whole system (WWDC22 10155).
- **Best fit for Porthole: Core Audio process taps (macOS 14.2+).** You can tap by PID, as a stereo mixdown of a set of processes. The tap description can be changed on a live tap. `CATapMuteBehavior.muted` or `.mutedWhenTapped` keeps the audio away from the speakers, so the human does not hear the agent's apps. The tap only needs the audio-only TCC grant (`NSAudioCaptureUsageDescription`). No drivers, no virtual devices, no AudioUnits.
- **Minimum versions:** SCK audio needs macOS 13.0. SCK microphone capture needs 15.0. Process taps need 14.2. Tap by bundle ID plus automatic process restore needs macOS 26.0.
- **Virtual devices (BlackHole and similar):** these are userspace HAL plug-ins. They need an admin install and a coreaudiod restart. macOS has **no public API to send another app's output to a chosen device**, so they only help with apps that let you pick an output device. They remain the standard way to **inject** audio into an app as a fake microphone.
- **AUHAL / AudioUnits:** not relevant for capturing another app's audio. Drop that thread.

### A. ScreenCaptureKit audio

**Configuration** (Apple docs, `SCStreamConfiguration`):
- `capturesAudio` (macOS 13.0): "A stream doesn't capture audio by default." ([doc](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/capturesaudio))
- `excludesCurrentProcessAudio` (13.0): leaves out the capturing app's own audio ([doc](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/excludescurrentprocessaudio)).
- `sampleRate` (13.0): only 8000, 16000, 24000 or 48000. Any other value gives 48 kHz ([doc](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/samplerate)).
- `channelCount` (13.0): 1 or 2 only. The default is stereo ([doc](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/channelcount)).
- WWDC22 says the same: "audio samples up to 48kHz stereo" ([Meet ScreenCaptureKit, WWDC22 10156](https://developer.apple.com/videos/play/wwdc2022/10156/)).
- The microphone arrived in macOS 15.0: `captureMicrophone`, `microphoneCaptureDeviceID`, and `SCStreamOutputType.microphone` ([captureMicrophone](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/capturemicrophone), [microphoneCaptureDeviceID](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/microphonecapturedeviceid), [.microphone](https://developer.apple.com/documentation/screencapturekit/scstreamoutputtype/microphone)). WWDC24 describes three media types on one stream: "screen, system audio, and microphone" ([WWDC24 10088](https://developer.apple.com/videos/play/wwdc2024/10088/)). This path only captures the physical mic. It cannot feed audio into an app.

**How the filter sets the audio scope.** The quotes are from [Take ScreenCaptureKit to the next level, WWDC22 10155](https://developer.apple.com/videos/play/wwdc2022/10155/):
- "ScreenCaptureKit's audio capture policy on the other hand always works at the app level."
- "When a single window filter is used, all the audio content from the application that contains the window will be captured, even from those windows that are not present in the video output."
- Display with included apps: "the audio output includes all the soundtracks from Keynote and Safari apps."
- Display with excluded apps: "If these removed apps include any audio, their audio will be removed from the audio output." Also, "excluding audio from a single Safari window is the equivalent to removing audio tracks for all Safari apps."
- WWDC22 10156 says the same thing in short: "audio capture can only be filtered at an application level."

So for each filter type:
- **`desktopIndependentWindow`:** the whole app's audio.
- **`display:includingApplications:exceptingWindows:`:** the union of those apps' audio.
- **`display:excludingApplications:`:** the system mix minus those apps.

No filter type gives per-window audio. [WWDC23 "What's new in ScreenCaptureKit"](https://developer.apple.com/videos/play/wwdc2023/10136/) adds no audio-scope features; its transcript only mentions audio when talking about A/V sync.

**Known limitations**
1. **Audio needs a video output too.** If a stream has only an `.audio` output, it logs "stream output NOT found. Dropping frame". The accepted answer in the forum thread says to use process taps instead ([forum 718279](https://developer.apple.com/forums/thread/718279), not an Apple engineer). OBS's audio-only SCK source adds "a dummy video stream output to silence errors from SCK" ([obs-studio mac-sck-audio-capture.m](https://github.com/obsproject/obs-studio/blob/master/plugins/mac-capture/mac-sck-audio-capture.m), around lines 128-145). OBS also uses `initWithDisplay:includingApplications:exceptingWindows:` with `setExcludesCurrentProcessAudio:TRUE`.
2. **SCK never mutes anything.** It only observes, so the human keeps hearing the app. No doc mentions a mute option (inferred from the API surface).
3. **Permission:** capture needs the Screen & System Audio Recording grant (WWDC22 10156: "the choice will be stored in the Screen Recording privacy setting").
4. **Helper processes.** App-level scoping follows `SCRunningApplication`. **UNVERIFIED:** whether audio played by a helper process is attributed to the parent app. Chromium plays audio from a separate audio-service process on Mac ([chromium services/audio/README.md](https://chromium.googlesource.com/chromium/src/+/HEAD/services/audio/README.md)).
5. **Output devices and spatial audio.** **UNVERIFIED:** whether SCK captures audio that goes to non-default or exclusive (hog-mode) devices, and how it handles spatial or multichannel audio (above 2 channels is not supported, per `channelCount`). No Apple source found.

### B. Core Audio process taps (macOS 14.2+)

**API surface.** Headers `AudioHardwareTapping.h`, `CATapDescription.h` and `AudioHardware.h` ([SDK 15.5 headers](https://github.com/alexey-lysiuk/macos-sdk/tree/main/MacOSX15.5.sdk/System/Library/Frameworks/CoreAudio.framework/Versions/A/Headers)):
- `AudioHardwareCreateProcessTap(CATapDescription*, AudioObjectID*)` and `AudioHardwareDestroyProcessTap` are `API_AVAILABLE(macos(14.2))` (AudioHardwareTapping.h; [doc](https://developer.apple.com/documentation/coreaudio/audiohardwarecreateprocesstap(_:_:))). The `CATapDescription` class shows `macos(12.0)` and the mute enum shows `macos(13.0)`, but taps can only be created from 14.2. The doc pages show the same mixed availability.
- `CATapDescription` initializers: `initStereoMixdownOfProcesses:`, `initStereoGlobalTapButExcludeProcesses:`, `initMonoMixdownOfProcesses:`, `initMonoGlobalTapButExcludeProcesses:`, `initWithProcesses:andDeviceUID:withStream:`, and `initExcludingProcesses:andDeviceUID:withStream:`.
  - Every one takes **HAL process AudioObjectIDs, not PIDs**.
  - For the stereo inits: "Mono sources will be duplicated in both right and left channels."
  - For the device/stream variants: "The format of the tap will match the format of this stream."
- `CATapDescription` properties:
  - `processes`.
  - `exclusive`: "tap all processes except the process listed".
  - `mono` and `mixdown`.
  - `privateTap`: "only visible to the client process that created the tap".
  - `muteBehavior`, `deviceUID`, `stream`, `name`, `UUID`.
  - macOS 26.0 adds `bundleIDs` and `processRestoreEnabled`: "save tapped processes by bundle ID when they exit, and restore them to the tap when they start up again" (MacOSX26.5 SDK `CATapDescription.h`; [bundleIDs doc](https://developer.apple.com/documentation/coreaudio/catapdescription/bundleids), [isProcessRestoreEnabled doc](https://developer.apple.com/documentation/coreaudio/catapdescription/isprocessrestoreenabled)).
- **Going from PID to process object:**
  - `kAudioHardwarePropertyTranslatePIDToProcessObject` ('id2p') takes the PID as its qualifier and returns `kAudioObjectUnknown`, not an error, if that PID is not a HAL client.
  - `kAudioHardwarePropertyProcessObjectList` ('prs#') lists "all client processes currently connected to the system".
  - Process properties: `kAudioProcessPropertyPID`, `...BundleID`, `...Devices`, `...IsRunning`, `...IsRunningInput` and `...IsRunningOutput` (AudioHardware.h lines ~584-596 and 1925-1975).
  - **Consequence:** a process that has not yet opened audio has no process object, so Porthole cannot tap it until it connects. Porthole must listen for changes to `ProcessObjectList`.
- **Aggregate device:**
  - `kAudioAggregateDeviceTapListKey` ("taps") holds dictionaries keyed by `kAudioSubTapUIDKey` ("uid"), with an optional `kAudioSubTapDriftCompensationKey`.
  - `kAudioAggregateDeviceTapAutoStartKey` delays `AudioDeviceStart` "until a tapped process begins receiving its first audio". It requires the private key.
  - `kAudioAggregateDevicePropertyTapList` ('tap#') lets you add or remove taps on a live aggregate.
  - `kAudioTapPropertyFormat` gives the tap's ASBD (AudioHardware.h ~1612-1690, 1843-1920, 1995-2018).
- **Live retargeting:** `kAudioTapPropertyDescription` is "The CATapDescription used to initially create this tap. This property can be used to **modify and set the description of an existing tap**" (AudioHardware.h ~2006). macOS 15 adds the Swift wrapper `AudioHardwareTap.setDescription(_:)` ([doc](https://developer.apple.com/documentation/coreaudio/audiohardwaretap)).
  - **Answer:** yes, PIDs can be added or removed without tearing down the tap.
  - **UNVERIFIED:** whether updating the description causes a glitch or dropout in the running IOProc.

**Call sequence.** Apple's sample ["Capturing system audio with Core Audio taps"](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps) builds a `CATapDescription`, calls `AudioHardwareCreateProcessTap`, reads `kAudioTapPropertyUID`, creates an aggregate, and appends the tap UID via `kAudioAggregateDevicePropertyTapList`. The same page says taps "can also mute the process output so that the process will no longer play to the speaker or selected audio device, and all process output will go to the tap."

[insidegui/AudioCap](https://github.com/insidegui/AudioCap) (`AudioCap/ProcessTap/ProcessTap.swift` lines ~92-157) does the following:
1. `CATapDescription(stereoMixdownOfProcesses: [objectID])`, with `muteBehavior = .mutedWhenTapped` or `.unmuted`.
2. `AudioHardwareCreateProcessTap`.
3. An aggregate with `kAudioAggregateDeviceMainSubDeviceKey` set to the default system output UID, `IsPrivate: true`, `TapAutoStart: true`, and `TapList: [{uid, drift: true}]`.
4. `AudioHardwareCreateAggregateDevice`.
5. `AudioDeviceCreateIOProcIDWithBlock`, then `AudioDeviceStart`.

**Mute behaviour** (`CATapDescription.h`; [CATapMuteBehavior doc](https://developer.apple.com/documentation/coreaudio/catapmutebehavior)):
- `CATapUnmuted = 0`: "captured by the tap and also sent to the audio hardware". This is the default.
- `CATapMuted = 1`: "captured by the tap but no audio is sent from the process to the audio hardware".
- `CATapMutedWhenTapped = 2`: "sent to the audio hardware until the tap is read by another audio client. For the duration of the read activity on the tap no audio is sent to the audio hardware."

**What this means for Porthole:**
- Muting is per tap, so it covers whatever processes the tap lists.
- A tap listing only the agent's PIDs with `.muted` makes those apps silent to the human, while their audio is still captured and everything else plays normally.
- `.mutedWhenTapped` falls back to audible when Porthole is not reading, which is a safer default if the daemon crashes. `.muted` stays silent as long as the tap exists.
- **UNVERIFIED:** what happens to a `.muted` tap if the creating process dies. It is probably destroyed along with that process when `privateTap` is set.

**Permission:**
- `NSAudioCaptureUsageDescription` (macOS 14.2+) is "A message that tells people why your app is requesting access to capture system audio" ([doc](https://developer.apple.com/documentation/bundleresources/information-property-list/nsaudiocaptureusagedescription)).
- The sample says the prompt appears "The first time you start recording from an aggregate device that contains a tap".
- There is no public preflight or request API. AudioCap uses the TCC SPI with service `kTCCServiceAudioCapture` (AudioRecordingPermission.swift; README: "There's no public API to request audio recording permission").
- Apple Support says users can let apps "record both your screen and audio, or just your audio" ([Apple Support mchld6aa7d23](https://support.apple.com/en-us/guide/mac-help/mchld6aa7d23/mac)). This is the "System Audio Recording Only" grant. **UNVERIFIED:** the exact label in the UI and which macOS versions show it.
- Porthole already needs to run from an `.app` bundle to get TCC prompts (`crates/porthole-adapter-macos/src/permissions.rs` ~139-155). The key belongs in `apps/macos/bundle/Info.plist`.

**Format.**
- Mixdown taps are stereo or mono float PCM. Read the actual ASBD from `kAudioTapPropertyFormat`.
- **UNVERIFIED:** whether the mixdown sample rate always follows the main sub-device's nominal rate. Device/stream taps match that stream (header).

**Known issue.** There is an unanswered report of taps delivering all-zero buffers during long sessions ([forum 825780](https://developer.apple.com/forums/thread/825780), no Apple reply).

### C. Virtual audio devices

**AudioServerPlugIn (HAL plug-in).** These run in user space; no kext is involved.
- `AudioServerPlugIn.h`: "An AudioServerPlugIn is a CFPlugIn that is loaded by the host process as a driver. The plug-in bundle is installed in /Library/Audio/Plug-Ins/HAL." It "operates in its own process separate from the system daemon", and "the host process is sandboxed."
- Apple's sample [Creating an Audio Server Driver Plug-in](https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in) says: "Install the sample's `.driver` bundle to `/Library/Audio/Plug-Ins/HAL` and reboot."
- A driver for real hardware can pair the plug-in with a DriverKit extension. That needs DriverKit entitlements, or SIP disabled for ad-hoc signing ([sample](https://developer.apple.com/documentation/coreaudio/building-an-audio-server-plug-in-and-driver-extension)). A purely virtual device does not need that.

**BlackHole** ([README](https://github.com/ExistentialAudio/BlackHole/blob/master/README.md)):
- "virtual audio loopback driver ... zero additional latency".
- Comes in 2, 16, 64, 128 and 256 channel builds, at 8 kHz to 768 kHz.
- Install by copying into `/Library/Audio/Plug-Ins/HAL` and running `sudo killall -9 coreaudiod`, or with `brew install blackhole-2ch`.
- The installer is signed and notarized (`Installer/create_installer.sh`).
- **Implication:** installing needs admin rights and restarts audio for the human too.

**Loopback (Rogue Amoeba).** On macOS 14.5+ it "uses new technology to capture audio" through a background "Audio Routing Kit (ARK)" that needs "System Audio Access permission" ([KB](https://rogueamoeba.com/support/knowledgebase/?showArticle=Misc-ARK-Plugin-Audio-Capture-Details&product=Loopback)). **UNVERIFIED:** that ARK is built on public process taps. The permission name suggests it is.

**Per-app output routing: there is no public API.**
- `kAudioHardwarePropertyDefaultOutputDevice` is a property of the system object: "The AudioObjectID of the default output AudioDevice" (AudioHardware.h ~476).
- The header treats default devices as **per-user** preferences, not per-process. `kAudioHardwarePropertyUserIDChanged` makes the HAL "flush all its cached per-user preferences such as the default devices."
- The `kAudioProcessProperty*` selectors are all readable status values: PID, bundle ID, devices, IsRunning*. The header documents none as settable. `kAudioProcessPropertyDevices` reports "the devices currently used by the process"; it does not choose them.
- `kAudioHardwarePropertyProcessIsAudible` ('pmut') and `...ProcessInputMute` apply only to "the process", meaning the calling client itself. They are not a handle on another app.
- **Conclusion:** Porthole cannot give its launched app a different default output device. **UNVERIFIED:** that no private or environment-variable mechanism exists; none is documented.
- The only per-app route is an app's own output-device setting, for apps that expose one. The Core Audio overview says the default output unit follows the user's choice ([Core Audio Overview, Common Tasks](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/ARoadmaptoCommonTasks/ARoadmaptoCommonTasks.html)).

### D. Audio Units / AUHAL: not relevant

- AUHAL (`kAudioUnitType_Output` / `kAudioUnitSubType_HALOutput`) is how *your own process* does I/O with one device. "An application can use the ... AudioOutputUnit to interface to a single audio device. The AUHAL can be used for input and output to an audio device" ([TN2091](https://developer.apple.com/library/archive/technotes/tn2091/_index.html)).
- It cannot see another process's output. It only becomes useful once that audio is exposed as an *input device*: a tap aggregate, or BlackHole's input side. At that point, a raw `AudioDeviceCreateIOProcIDWithBlock` (as AudioCap uses) or `AVAudioEngine` does the same job.
- **Close this thread.**

### E. Isolation model for an "agent desktop"

| Option | Permission | Human hears agent? | Scope | Format / latency | Min macOS | Headless daemon? |
|---|---|---|---|---|---|---|
| 1. Process tap, PIDs of the agent tree, `.muted` or `.mutedWhenTapped` | System audio recording (`NSAudioCaptureUsageDescription`) | **No** (header semantics) | Exactly the listed processes; editable live via `kAudioTapPropertyDescription` | Float PCM from `kAudioTapPropertyFormat`, IOProc buffer latency (**UNVERIFIED** figures) | 14.2 (bundleID/restore: 26.0) | Needs a bundled app identity for TCC; no window needed (**UNVERIFIED** for pure LaunchAgent binaries) |
| 2. Virtual device as default output for the agent process only | Admin install; no TCC for the device; a tap or input needs mic/audio TCC | No, if routing works | **Not possible**: no per-process default output (§C). Only apps with an in-app device picker. | BlackHole: zero extra latency, any rate | any | Install needs admin and a coreaudiod restart |
| 3. Separate user session | Session-dependent | Probably not while the session is inactive (**UNVERIFIED**) | Everything in that session | n/a | any | **UNVERIFIED** |
| 4. SCK app-scoped (`includingApplications`) | Screen & System Audio Recording | **Yes** (SCK never mutes) | App level, never window level | ≤48 kHz, ≤2 ch, CMSampleBuffers alongside video | 13.0 | Porthole already runs SCK under TCC; needs a dummy screen output |

Notes on option 3:
- The HAL exposes `kAudioHardwarePropertyUserSessionIsActiveOrHeadless`: "the login session of the user of the process is either an active console session or a headless session" (AudioHardware.h ~569). This suggests the HAL gates audio by session state.
- **UNVERIFIED:** no Apple document was found saying that audio from a background fast-user-switch session is muted, or that coreaudiod keeps separate state per session. The default devices are per-user (`UserIDChanged`).
- This is a heavyweight option with unclear capture semantics.

**Process-tree caveat for option 1.** Browsers and Electron apps play audio from a helper process: Chromium's audio service runs "In a separate process on ... Mac" ([chromium README](https://chromium.googlesource.com/chromium/src/+/HEAD/services/audio/README.md)). Porthole must therefore:
- tap every descendant PID of the launched app, not just the main PID;
- watch `kAudioHardwarePropertyProcessObjectList` and re-set the tap description whenever a new descendant connects.

### F. Input side (injecting audio into an agent app)

To feed audio *into* an app, the standard approach is a virtual loopback device such as BlackHole. Porthole plays audio to the device's output side, and the agent's app selects that device as its microphone (BlackHole README: "Input audio from the corresponding output channels"). This has the same install cost as §C: an admin install into `/Library/Audio/Plug-Ins/HAL` and a coreaudiod restart. The app must also either pick that device itself or run while the device is the system default input. `kAudioHardwarePropertyDefaultInputDevice` is global, so changing it affects the human too. The app still needs microphone TCC.

SCK's macOS 15 microphone support (`captureMicrophone`, `.microphone`) only *records* a physical input device into the capture stream ([WWDC24 10088](https://developer.apple.com/videos/play/wwdc2024/10088/)). It cannot inject audio. Process taps also only capture output. No public Apple API was found for giving one process a private virtual microphone (**UNVERIFIED** as a negative).

### Recommendation candidates

1. **Process tap of the agent process tree, `muteBehavior = .muted`, private tap plus private aggregate (recommended).**
   - Risks:
     - Only processes already connected to the HAL can be tapped. Mitigate with a listener on `ProcessObjectList`.
     - Helper-process coverage.
     - A TCC prompt from a daemon context.
     - The zero-buffer reports in forum 825780.
     - Behaviour on macOS 14.2-14.3 (AudioCap targets 14.4; the header says 14.2).
   - Experiment: launch Safari and Chrome playing a tone via Porthole. Tap the tree with `.muted`, and confirm (a) the speakers are silent, (b) the tapped buffers hold non-zero samples, (c) a system alert sound still plays, and (d) adding a PID via `kAudioTapPropertyDescription` mid-stream works without a restart and records how long any glitch lasts.
2. **Same as 1 on macOS 26+, using `bundleIDs` and `processRestoreEnabled`.**
   - Risk: it is scoped by bundle ID, so the human's own copy of the same app (for example Safari) would also be tapped and muted.
   - Experiment: run the same bundle for the human and the agent at the same time and check whether the human's copy gets muted.
3. **SCK `display:includingApplications:` audio alongside the existing video stream.**
   - Risk: the human hears everything, and scoping is per app, not per window.
   - Experiment: add `.audio` output to the existing window stream and confirm that audio from a second window of the same app shows up.
4. **BlackHole, only for microphone injection.**
   - Risk: admin install, a coreaudiod restart, and a global default input.
   - Experiment: have Porthole play a WAV to BlackHole's output, and check that a Porthole-launched app with BlackHole chosen as its input receives it.

### Open questions

- Does `.muted` survive or release if Porthole crashes? Is a leaked tap cleaned up when the process exits (`privateTap`)?
- Can a LaunchAgent binary inside Porthole's `.app` trigger and hold the `kTCCServiceAudioCapture` grant, and is it attributed to the bundle or to the responsible process?
- Is the tap's mixdown rate fixed to the main sub-device? What happens to the tap when the human changes the default output device (for example, plugs in AirPods)?
- Does SCK attribute helper-process audio (Chromium audio service) to the parent `SCRunningApplication`?
- What does an inactive or background user session do to audio (option 3)?
- What end-to-end latency does a tap to aggregate to IOProc chain have? Measure it; no Apple figure was found.

## Thread 3: Linux and Windows comparison points

Researched 2026-09-24 against upstream source and official docs. "Source-level" means we read the code but did not run it. Items marked **UNVERIFIED** have no primary-source confirmation.

### Summary

- **Linux has one clean answer: the agent gets its own compositor.** When porthole starts the compositor, it is the compositor's trusted client. It can capture with Wayland protocols or a PipeWire node the compositor publishes, and inject input through virtual-input protocols or EIS. No portal dialog is involved, and the human's seat, focus and cursor are never touched. Gamescope (headless backend, a PipeWire `Video/Source` node, its own EIS socket) and Weston (`pipewire` backend) come closest to working out of the box. Sway, labwc and cage (wlroots) expose screencopy plus virtual keyboard and pointer. A nested `kwin_wayland --virtual` would let porthole reuse its existing KWin screencast code, minus the portal.
- **KWin surprise (source-level):** current KWin master blocks `zkde_screencast_unstable_v1`, `org_kde_kwin_fake_input` and `org_kde_plasma_window_management` only for *sandboxed* clients. An unsandboxed porthole could bind them directly, even in the human's session.
- **Per-app audio on Linux is solved by PipeWire routing:** give each agent a null sink, point the launched process at it with `PIPEWIRE_NODE`/`PULSE_SINK`, and record the sink's monitor. The human's default device never carries the audio.
- **Windows has no equivalent of an agent-owned compositor.** Apollo/Sunshine create a real extra monitor with an IddCx driver, but it is part of the human's desktop, so cursor and focus stay shared. Input goes to the one input desktop. Per-process audio *capture* works through process-loopback activation, but no documented API stops the human from *hearing* that audio.

---

### A. Headless or nested compositors as an "agent desktop" (Linux)

| Compositor | How to run headless | Dialog-free capture | Input injection | Source |
|---|---|---|---|---|
| wlroots (library) | `WLR_BACKENDS=headless`, `WLR_HEADLESS_OUTPUTS=N`, `WLR_LIBINPUT_NO_DEVICES=1`, GPU chosen with `WLR_RENDER_DRM_DEVICE` | depends on the compositor | depends on the compositor | wlroots `docs/env_vars.md` (gitlab.freedesktop.org/wlroots/wlroots) |
| sway | wlroots env vars above; `swaymsg create_output` adds a 1920x1080 headless output (code comment: "intended for developer use only") | `wlr-screencopy`, `ext-image-copy-capture-v1`, `ext-image-capture-source-v1`, `wlr-export-dmabuf` | `virtual-keyboard-v1`, `wlr-virtual-pointer-v1`, transient-seat | github.com/swaywm/sway `sway/commands/create_output.c` (`wlr_headless_add_output(backend,1920,1080)`); `sway/server.c`; `sway/input/input-manager.c` |
| labwc | wlroots env vars | screencopy, ext-image-copy-capture (output and foreign-toplevel sources), export-dmabuf, `ext-foreign-toplevel-list` | virtual pointer and keyboard | github.com/labwc/labwc `src/server.c`, `src/seat.c` |
| cage (kiosk: one maximised app) | wlroots autocreate (`wlr_backend_autocreate`), so the headless env vars apply | `wlr-screencopy`, `wlr-export-dmabuf` only (no ext-image-copy-capture in `cage.c`) | virtual keyboard and pointer managers | github.com/cage-kiosk/cage `cage.c` (`wlr_screencopy_manager_v1_create`, `wlr_virtual_keyboard_manager_v1_create`) |
| weston | `--backend=headless` ("can be used to capture Weston outputs"); `--backend=pipewire` "creates a PipeWire node for each output" | the PipeWire backend publishes a node per output | **UNVERIFIED** (no virtual-input protocol found; the RDP backend gives "each connecting client its own seat") | weston `man/weston.man` (gitlab.freedesktop.org/wayland/weston) |
| gamescope | `--backend headless` ("no window, no DRM output"); `--backend wayland`/`sdl` for nested use | PipeWire stream `"gamescope"`, `PW_KEY_MEDIA_CLASS "Video/Source"`, dmabuf/Vulkan buffers | runs its own **libeis** server on socket `<wl_display>-ei` and exports `LIBEI_SOCKET` to children; also a virtual keyboard device | github.com/ValveSoftware/gamescope `src/main.cpp`, `src/pipewire.cpp`, `src/wlserver.cpp` (L2322–2334) |
| KWin | `kwin_wayland --virtual` ("Render to a virtual framebuffer", `KWin::VirtualBackend`), plus `--socket`, `--width/--height`, `--output-count`, `--no-global-shortcuts`, `--no-lockscreen`, `--exit-with-session` | `zkde_screencast_unstable_v1` gives a KWin-produced PipeWire node, per output, per window or per virtual output | `org_kde_kwin_fake_input`; EIS through D-Bus `org.kde.KWin.EIS.RemoteDesktop.connectToEIS` | invent.kde.org/plasma/kwin `src/main_wayland.cpp`, `src/backends/virtual/`, `src/plugins/screencast/screencastmanager.cpp`, `src/plugins/eis/eisbackend.cpp` |

**Security model: who needs a portal?**

- **wlroots compositors.** Privileged globals (screencopy, image-copy-capture, virtual keyboard and pointer, foreign-toplevel) are hidden only from clients that carry a `wp_security_context_v1`, i.e. Flatpak-style sandboxes. See sway `sway/server.c` `is_privileged()`/`filter_global` ("Restrict usage of privileged protocols to unsandboxed clients") and labwc `src/server.c` `allow_for_sandbox`. A native process started by porthole on the agent compositor's socket gets them directly, with no dialog.
- **KWin master.** `KWinDisplay::allowInterface` in `src/wayland_server.cpp` refuses `restrictedInterfaces` only when `client->isSandboxed()`. That set is `org_kde_plasma_window_management`, `org_kde_kwin_fake_input`, `zkde_screencast_unstable_v1`, `kde_lockscreen_overlay_v1`, `wp_security_context_manager_v1` and `ext_data_control_manager_v1`. We did not find which Plasma release first shipped this rule; the older `X-KDE-Wayland-Interfaces` desktop-file allow-list is not present on master (**UNVERIFIED** release boundary). So porthole's current portal ScreenCast/RemoteDesktop path (`crates/porthole-adapter-kwin/src/screencast.rs`, `persist_mode`) could in principle go straight to `zkde_screencast_unstable_v1`. xdg-desktop-portal-kde already does that internally; per porthole's `docs/2026-07-06-pipewire-lease-gated-handback.md`, KWin is the PipeWire producer either way.
- **KWin EIS.** `EisBackend::connectToEIS(capabilities, cookie)` returns an EIS fd to any session-bus caller, and the method body has no caller check. xdg-desktop-portal-kde `src/remotedesktop.cpp` calls it after the user approves. A nested KWin registers this object on whatever session bus it inherits, so each agent KWin needs its own bus (e.g. `dbus-run-session`) to avoid colliding with the human's KWin. **UNVERIFIED:** we have not tested that a nested KWin loads the eis and screencast plugins.
- **KWin virtual outputs in the *human's* session.** `zkde_screencast` `stream_virtual_output` makes KWin create an output through `outputBackend()->createVirtualOutput(...)` and remove it when the stream closes (`screencastmanager.cpp` L73–88). That gives an off-screen surface, but it joins the human's workspace, seat and cursor, which is the same limitation as Apollo on Windows.
- **Input injection without touching the human.** In the agent's own compositor, `virtual-keyboard`/`virtual-pointer` or EIS only affect that compositor's seat. `ydotool`/uinput create kernel devices that every libinput-backed compositor, including the human's, will pick up (**UNVERIFIED** here; follows from uinput semantics). Avoid them.

### B. Wayland protocol status

- **`ext-image-copy-capture-v1`** (staging; first commit 2024-08-10 "ext-image-copy-capture-v1: new protocol"): a capture session advertises shm/dmabuf constraints, then the client attaches a buffer and requests a capture. Source: wayland-protocols `staging/ext-image-copy-capture/ext-image-copy-capture-v1.xml`.
- **`ext-image-capture-source-v1`**: an opaque source, created from a `wl_output` (`ext_output_image_capture_source_manager_v1`) or from a foreign toplevel (`ext_foreign_toplevel_image_capture_source_manager_v1`), so it can capture **a single window**. Source: `staging/ext-image-capture-source/…xml`.
- **`ext-foreign-toplevel-list-v1`** (2023-04-25): lists toplevels. Its `identifier` event gives a unique string of up to 32 bytes that is never reused, "useful for command line tools or privileged clients which may need to reference an exact toplevel across processes".
- **`xdg-toplevel-tag-v1`** (staging, 2025-04-02): `set_toplevel_tag` and `set_toplevel_description` are set **by the application itself**, not by porthole. The tag is untranslated and "does not need to be unique". It helps only when the app sets it, so it is not an identity porthole can assign.
- **`wp-tearing-control-v1`** (2022-11): only a presentation hint, not relevant to isolation.
- **libei / EIS**: EIS runs inside the compositor, and a libei "sender" connects over a UNIX socket, either through `LIBEI_SOCKET` or through the portal via liboeffis (libinput.pages.freedesktop.org/libei). **RemoteDesktop portal v2 `ConnectToEIS`** "must be called after Start". After that, input must go only over EIS and the `Notify*` methods return errors. `persist_mode=2` and `restore_token` let a grant survive restarts (flatpak.github.io/xdg-desktop-portal, `org.freedesktop.portal.RemoteDesktop`).
- **KWin support (master, source-level):**
  - `xdg-toplevel-tag`: yes (`src/wayland/xdgtopleveltag_v1.cpp`, first commit 2025-04-28).
  - `tearing_control_v1`: yes (2022-12-05).
  - libeis backend plugin: yes (`src/plugins/eis`, 2024-04-15).
  - `ext-image-copy-capture` / `ext-image-capture-source` / `ext-foreign-toplevel-list`: **not found** in `src/wayland/` or `wayland_server.cpp` on master. KWin still uses `zkde_screencast_unstable_v1` (v6: `stream_output`, `stream_window`, `stream_virtual_output(_with_description)`, `stream_region`).
  - Window identity: `org_kde_plasma_window_management` v21, which has `pid_changed`, `app_id_changed`, `resource_name_changed`, `get_window_by_uuid` and `send_to_output` (invent.kde.org/libraries/plasma-wayland-protocols `src/protocols/plasma-window-management.xml`).
  - Mapping these commits to Plasma release versions is **UNVERIFIED**.

### C. Per-app audio on Linux (PipeWire)

1. **Create a per-agent null sink** in one of three ways:
   - PipeWire-native: `pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=porthole-agent-X media.class=Audio/Sink object.linger=true audio.position=[FL FR] monitor.channel-volumes=true }'`
   - config fragment: `context.objects` with `factory = adapter`
   - pulse compatibility: `pactl load-module module-null-sink media.class=Audio/Sink sink_name=…`

   "The sink by itself will not play any audio. It will have monitor ports and a monitor source" (PipeWire wiki *Virtual-Devices*, gitlab.freedesktop.org/pipewire/pipewire/-/wikis/Virtual-Devices; `pipewire-pulse-modules(7)`). Nothing reaches the human's speakers unless someone links it there.
2. **Route the launched app to it:**
   - Native PipeWire clients: `PIPEWIRE_NODE` "Makes a stream connect to a specific object.serial or node.name"; `PIPEWIRE_PROPS` "Adds extra properties to a stream" (pipewire(1), docs.pipewire.org/page_man_pipewire_1.html).
   - libpulse clients (most apps through pipewire-pulse): `$PULSE_SINK` "takes precedence" over the default sink (pulseaudio `man/pulse-client.conf.5.xml.in`).
   - Stream properties: `target.object = <node.name|object.serial>`. `node.dont-fallback` means don't fall back to the default target if linking fails. `node.dont-reconnect` destroys the stream if the target disappears. Together they prevent leaks to the human's device (`pipewire-props(7)`).
   - Policy-side routing: WirePlumber `stream.rules` (match on stream properties, e.g. `application.name`; `update-props`), plus `state.restore-target = false` so WirePlumber's remembered target does not override the route (pipewire.pages.freedesktop.org/wireplumber/daemon/configuration/stream.html). Setting `target.object` through `update-props` is plausible but **UNVERIFIED**.
3. **Capture:** open a `pw_stream` targeting the sink with `stream.capture.sink = true` (`PW_KEY_STREAM_CAPTURE_SINK`, `src/pipewire/keys.h`), or record its monitor (`pw-record --target <node.name|serial>`, pw-cat(1)). `pw-link` can wire ports by hand.
4. **Security caveats:**
   - Only `pipewire.*` client properties are trustworthy, because clients set everything else freely.
   - `pipewire.sec.pid` "for PulseAudio applications, this is the PID of the pipewire-pulse process" (`pipewire-props(7)`), so PID-based matching fails for most apps. Route at launch through env vars instead.
   - Flatpak clients are classified through `pipewire.access` / `pipewire.sec.flatpak` and get WirePlumber `access.rules` default permissions (e.g. `rx`) (wireplumber `configuration/access.html`). A Flatpak app may be unable to see a custom sink, and env vars may not reach it (**UNVERIFIED**).

### D. Per-agent users, sessions and namespaces as the isolation unit

A separate PipeWire instance per agent is directly supported. `PIPEWIRE_RUNTIME_DIR`/`XDG_RUNTIME_DIR` pick the socket directory, `PIPEWIRE_CORE` names the server socket, `PIPEWIRE_REMOTE` picks the socket a client connects to, and `PIPEWIRE_DAEMON` turns a process into a server (pipewire(1)). WirePlumber documents running multiple instances (pipewire.pages.freedesktop.org/wireplumber/daemon/multi_instance.html). Running each agent as its own Unix user with its own `XDG_RUNTIME_DIR`, D-Bus session, PipeWire and headless compositor gives separation enforced by filesystem permissions. Porthole would then connect to that agent's compositor and PipeWire sockets. The cost: the agent's audio and video never appear in the human's graph, so porthole must bridge them.

A seat is only needed to drive real DRM/input hardware. seatd/libseat "mediat[es] access to shared devices (graphics, input)" and supports seatd, (e)logind or an embedded seatd, selectable with `LIBSEAT_BACKEND` (git.sr.ht/~kennylevinsen/seatd README). A headless wlroots, Weston or gamescope instance renders through a GPU render node and has no libinput devices (`WLR_LIBINPUT_NO_DEVICES`). It therefore should need no logind seat at all, which avoids multiseat configuration entirely. This is an inference from the headless backend design, **UNVERIFIED** by a test.

### E. Windows: what Apollo and Sunshine do

- **Virtual display.** Apollo "uses SudoVDA for virtual display … created upon the stream starts and removed once the app quits" and gives each client a fixed monitor identity. It "works just like any physically attached monitors". "Headless mode" means rendering on a chosen GPU with no dummy plug (github.com/ClassicOldSong/Apollo README). SudoVDA is based on Microsoft's Indirect Display sample and the IddCx class extension; registry keys include `maxMonitors` (default 10) and `gpuName` (github.com/SudoMaker/SudoVDA README).
- **What IddCx is.** A user-mode (UMDF) model "to support monitors that aren't connected to traditional GPU display outputs", including "virtual monitors". IddCx "provides the desktop image to encode in a DirectX surface" (learn.microsoft.com/…/display/indirect-display-driver-model-overview).
- **Consequence:** an IddCx monitor is part of the one desktop, so it is captured like any output. Sunshine's WGC path enumerates DXGI outputs and calls `IGraphicsCaptureItemInterop::CreateForMonitor`, disabling the border with `IsBorderRequired(false)` (github.com/LizardByte/Sunshine `src/platform/windows/display_wgc.cpp`). The same consequence means the human's cursor can move onto the monitor and windows can move between monitors.
- **Audio.** Sunshine finds "Steam Streaming Speakers" (or a configured `virtual_sink`) and makes it the *system default* with the undocumented `IPolicyConfig::SetDefaultEndpoint` (`src/platform/windows/PolicyConfig.h`, `audio.cpp` `set_sink`). It then captures with `AUDCLNT_STREAMFLAGS_LOOPBACK` (`audio.cpp` L393). That redirects *all* host audio, not one app's.
  - Microsoft: loopback captures "the system mix that is being played by the audio engine" and works only for shared-mode streams (learn.microsoft.com/…/coreaudio/loopback-recording).
  - Per-process: `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` and `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS{TargetProcessId, ProcessLoopbackMode}` includes or excludes a process *tree*. Minimum client: **Windows 10 Build 20348** per Microsoft Learn, not "2004" as sometimes stated. The ApplicationLoopback sample: "capture is not tied to a specific audio endpoint".
  - This captures the app but does **not** stop it playing on the human's device. There is no documented per-app output-routing API (**UNVERIFIED** absence; the Settings per-app device choice is not a public API).
- **Capture APIs.**
  - WGC `CreateForWindow`/`CreateForMonitor` through `IGraphicsCaptureItemInterop` needs Windows 10 1903 (build 18362) and no picker. The picker/consent flow and the yellow border are the documented default (learn.microsoft.com screen-capture; `IGraphicsCaptureItemInterop`).
  - Desktop Duplication (`IDXGIOutput1::DuplicateOutput`) works per output. It returns `E_ACCESSDENIED` without access to "the current desktop image (e.g. secure desktop)" and `DXGI_ERROR_UNSUPPORTED` across desktop switches. It is limited to four duplicating processes per session.
  - Porthole today uses `PrintWindow` + `SetForegroundWindow` + `SendInput` (`crates/porthole-adapter-windows/src/native.rs`).
- **Input and desktops.**
  - `SendInput` inserts events into "the keyboard or mouse input stream", the same one the human types into, and is subject to UIPI (learn.microsoft.com `SendInput`). Sunshine follows the input desktop with `OpenInputDesktop`/`SetThreadDesktop` (`src/platform/windows/misc.cpp`) and now builds on libvirtualhid (`input.cpp`).
  - `CreateDesktop` desktops are separate UI namespaces: "Window messages can be sent only between processes that are on the same desktop". But "only one of these desktops at a time is active … the *input desktop*, is the one that is currently visible to the user and that receives user input" (learn.microsoft.com/…/winstation/desktops).
  - A hidden desktop therefore gets no real input stream and is not what the DWM presents. Microsoft does not state that GPU/WGC capture of a non-input desktop fails; that limitation is **UNVERIFIED** (commonly observed, not documented).

### F. Comparison

| | Isolated display surface | Dialog-free capture of it | Input without stealing human focus | Per-app audio capture | Does the human see or hear it? |
|---|---|---|---|---|---|
| **Linux: agent-owned compositor** | headless sway/labwc/cage/weston/gamescope, or `kwin_wayland --virtual` | yes: screencopy / ext-image-copy-capture, gamescope or weston PipeWire node, direct `zkde_screencast` | yes: virtual-keyboard/pointer or EIS into *that* compositor | null sink + `PIPEWIRE_NODE`/`PULSE_SINK` + monitor capture | No and no |
| **Linux: human's KWin session (porthole today)** | only a KWin virtual output (shared workspace) | portal with a persisted grant; direct `zkde_screencast` possible for unsandboxed clients (source-level) | no: EIS/fake-input share the seat, focus and cursor | same PipeWire routing works | Sees windows unless placed on a virtual output; hears nothing with a null sink |
| **Windows** | IddCx virtual monitor (SudoVDA), still on the one desktop | yes: WGC interop or Desktop Duplication, no picker, border can be disabled | no: `SendInput` goes to the single input desktop; `CreateDesktop` isolates but gets no input stream or visible frames | process-tree loopback (build 20348+) | Can see it (cursor can reach the monitor); hears it unless *all* audio is redirected Sunshine-style |
| **macOS** | researched separately | — | — | — | — |

### What porthole could borrow

1. **A Linux "agent desktop" backend.** Porthole would spawn one headless compositor per agent session:
   - Launch: `WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c <minimal>` or `labwc`, or `gamescope --backend headless -W w -H h -- <app>`. Give each its own `WAYLAND_DISPLAY` socket and pass that plus `PIPEWIRE_NODE`/`PULSE_SINK` to the launched app.
   - Capture: bind `ext_image_copy_capture_manager_v1` with a dmabuf buffer pool, using `ext_foreign_toplevel_image_capture_source_manager_v1` for per-window frames. For gamescope, consume the `Video/Source` node through the existing Jackstay PipeWire consumer; the hold-until-release lease gating carries over unchanged.
   - Input: bind `zwp_virtual_keyboard_manager_v1`/`zwlr_virtual_pointer_manager_v1`, or connect libei to gamescope's `LIBEI_SOCKET`.
   - Identity: `ext-foreign-toplevel-list` `identifier` plus `app_id`, correlated with the PID porthole launched.
2. **The KWin variant (fastest reuse).** Run `dbus-run-session kwin_wayland --virtual --socket porthole-agent-N --no-global-shortcuts --no-lockscreen --width … --height …`. Replace the portal handshake with direct `zkde_screencast_unstable_v1.stream_window/stream_output`, and get input from `org.kde.KWin.EIS.RemoteDesktop.connectToEIS` on the nested bus. That removes the dialog-timeout blocker noted in #104/#108.
3. **Per-launch audio sink.** On `launch`, create `support.null-audio-sink` `node.name=porthole-<surface>` with `object.linger=false` tied to porthole's connection. Set `PIPEWIRE_NODE`, `PULSE_SINK`, and `PIPEWIRE_PROPS={node.dont-fallback=true}` in the child env. Record with `target.object=<sink>` + `stream.capture.sink=true`. Destroy the sink on close. Do not rely on `pipewire.sec.pid`.
4. **Windows.**
   - Add process-tree loopback capture keyed on the launched PID (build 20348 gate).
   - Evaluate WGC `CreateForWindow` (border off) instead of `PrintWindow` for capture that needs no foreground.
   - Optionally drive a SudoVDA/IddCx monitor per agent for placement: windows are moved to its coordinates so they are off the human's physical screens.
   - Treat input focus as unsolvable on one desktop. Document it and keep ADR-level consent, or consider a second interactive session (e.g. RDP to self). Whether that suits porthole is an open question.

### Open questions

- Does a nested `kwin_wayland --virtual` load the screencast and eis plugins and produce dmabuf PipeWire streams without a GPU-backed output? Which Plasma release dropped the `X-KDE-Wayland-Interfaces` gating?
- Does gamescope's PipeWire node carry per-app content, or the composited output only? It has a `gamescope_focus_appid` format property.
- GPU sharing: does each headless compositor need its own render-node allocation, and what is the memory cost for N agents?
- Clipboard, IME and portals *inside* the agent compositor. Apps that call portals (file chooser) will reach the human's xdg-desktop-portal unless a per-agent D-Bus session is used.
- Windows: is WGC capture of windows on a non-input `CreateDesktop` desktop, or in a disconnected session, possible at all? Microsoft does not document it.
- Windows: is there any supported per-process output-routing API, so that the human does not hear the agent without the global default switch?
