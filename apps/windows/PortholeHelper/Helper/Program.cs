using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text.Json;
using System.Threading;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Forms = System.Windows.Forms;

namespace Porthole.WindowsHelper;

public partial class PortholeHelperApp : Application
{
    [DllImport("user32.dll")]
    static extern bool SetForegroundWindow(IntPtr hwnd);

    public PortholeHelperApp()
    {
        Directory.CreateDirectory(evidence);
        UnhandledException += (_, e) => Record("startup_error", error: e.Message + Environment.NewLine + e.Exception.ToString());
        InitializeComponent();
    }
    TrayShell? tray;
    CancellationTokenSource? handoffCancellation;
    bool committed;
    bool unresolved;
    bool quitting;
    int? daemonPid;
    DateTime? daemonStarted;
    int? workerPid;
    DateTime? workerStarted;
    Mutex? instanceMutex;
    bool instanceMutexHeld;
    readonly string evidence = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Porthole", "helper");
    void Record(string status, int? exit = null, string? error = null)
    {
        Directory.CreateDirectory(evidence);
        var elevated = new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator);
        File.WriteAllText(Path.Combine(evidence, "status.json"), JsonSerializer.Serialize(new {
            status, exit, error, utc = DateTime.UtcNow, helper_pid = Environment.ProcessId,
            helper_path = Environment.ProcessPath, helper_elevated = elevated,
            session = Process.GetCurrentProcess().SessionId,
            daemon_pid = daemonPid, daemon_started = daemonStarted,
            winui = typeof(Application).Assembly.GetName().Version?.ToString()
        }, new JsonSerializerOptions { WriteIndented = true }));
    }
    void HandoffRecord(string status, bool pending, string? error = null, string? workerResult = null)
    {
        string path = Path.Combine(evidence, "handoff.json");
        string temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        string content = JsonSerializer.Serialize(new {
            status, unresolved = pending, error, worker_result = workerResult,
            utc = DateTime.UtcNow, daemon_pid = daemonPid, daemon_started = daemonStarted,
            worker_pid = workerPid, worker_started = workerStarted,
            windows_session = Process.GetCurrentProcess().SessionId
        });
        try {
            File.WriteAllText(temporary, content);
            File.Move(temporary, path, true);
        } finally {
            if (File.Exists(temporary)) File.Delete(temporary);
        }
        unresolved = pending;
        // The journal is authoritative. A secondary status-file failure must
        // not turn a recorded commit into a pre-commit error path.
        try { Record(status, error: error); }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }
    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        string user = WindowsIdentity.GetCurrent().User?.Value ?? throw new InvalidOperationException("Windows user identity missing");
        instanceMutex = new Mutex(false, "Local\\PortholeHelper." + user);
        try { instanceMutexHeld = instanceMutex.WaitOne(0); }
        catch (AbandonedMutexException) { instanceMutexHeld = true; }
        if (!instanceMutexHeld) { instanceMutex.Dispose(); Environment.Exit(0); }
        var view = new HandoffWindow();
        const int width = 420;
        const int height = 340;
        var presenter = OverlappedPresenter.CreateForToolWindow();
        presenter.IsResizable = false;
        presenter.SetBorderAndTitleBar(true, false);
        view.AppWindow.SetPresenter(presenter);
        view.AppWindow.Resize(new Windows.Graphics.SizeInt32(width, height));
        var info = view.Status;
        tray = new TrayShell(view.DispatcherQueue, point => {
            var work = Forms.Screen.FromPoint(point).WorkingArea;
            int x = Math.Clamp(point.X - width / 2, work.Left, Math.Max(work.Left, work.Right - width));
            int desiredY = point.Y < work.Top ? point.Y + 28 : point.Y - height - 28;
            int y = Math.Clamp(desiredY, work.Top, Math.Max(work.Top, work.Bottom - height));
            view.AppWindow.Move(new Windows.Graphics.PointInt32(x, y));
            view.AppWindow.Show();
            view.Activate();
            SetForegroundWindow(WinRT.Interop.WindowNative.GetWindowHandle(view));
        }, () => {
            quitting = true;
            view.Close();
        });
        view.AppWindow.Closing += (_, e) => {
            if (quitting) return;
            e.Cancel = true;
            if (!committed) handoffCancellation?.Cancel();
            view.AppWindow.Hide();
        };
        view.Activated += (_, e) => {
            if (e.WindowActivationState == WindowActivationState.Deactivated && handoffCancellation == null)
                view.AppWindow.Hide();
        };
        view.DismissRequested += () => {
            if (handoffCancellation == null) view.AppWindow.Hide();
        };
        try {
            var path = Path.Combine(evidence, "handoff.json");
            if (File.Exists(path)) {
                using var prior = JsonDocument.Parse(File.ReadAllText(path));
                unresolved = prior.RootElement.GetProperty("unresolved").GetBoolean();
            }
        } catch { unresolved = true; }
        view.Handoff.IsEnabled = !unresolved;
        if (unresolved) {
            view.InspectPrevious.Visibility = Visibility.Visible;
            info.Title = "Previous handoff needs inspection";
            info.Message = "An earlier commit has an uncertain outcome. Inspect the Windows session before another attempt.";
            info.Severity = InfoBarSeverity.Warning;
        }
        view.Closed += (_, _) => {
            handoffCancellation?.Cancel();
            tray?.Dispose();
            tray = null;
            if (instanceMutexHeld) { instanceMutex?.ReleaseMutex(); instanceMutexHeld = false; }
            instanceMutex?.Dispose();
        };
        view.CancelAttempt.Click += (_, _) => handoffCancellation?.Cancel();
        view.InspectPrevious.Click += (_, _) => InspectPrevious(view);
        view.AcknowledgePrevious.Click += (_, _) => AcknowledgePrevious(view);
        view.Handoff.Click += async (_, _) => await ExecuteHandoffAsync(view);
        view.Activate(); view.AppWindow.Hide(); Record("ready");
    }

    void InspectPrevious(HandoffWindow view)
    {
        if (!unresolved || handoffCancellation != null) return;
        view.AcknowledgePrevious.Visibility = Visibility.Collapsed;
        try {
            var inspection = HandoffRecovery.InspectCurrent();
            view.Status.Title = inspection.CanAcknowledge ? "Inspect the current session" : "Previous handoff still needs attention";
            view.Status.Message = inspection.Message + (inspection.CanAcknowledge
                ? " Check the desktop and Porthole before acknowledging this unresolved attempt. Inspection does not prove whether the earlier transfer succeeded."
                : " No new handoff is allowed.");
            view.Status.Severity = InfoBarSeverity.Warning;
            if (inspection.CanAcknowledge) view.AcknowledgePrevious.Visibility = Visibility.Visible;
        } catch (Exception error) {
            view.Status.Title = "Inspection could not complete";
            view.Status.Message = error.Message;
            view.Status.Severity = InfoBarSeverity.Warning;
        }
    }

    void AcknowledgePrevious(HandoffWindow view)
    {
        if (!unresolved || handoffCancellation != null || view.AcknowledgePrevious.Visibility != Visibility.Visible) return;
        try {
            // Recheck immediately before re-arming. Preserve the original
            // record, including malformed content, for later diagnosis.
            var inspection = HandoffRecovery.InspectCurrent();
            if (!inspection.CanAcknowledge) {
                view.AcknowledgePrevious.Visibility = Visibility.Collapsed;
                view.Status.Title = "Previous handoff still needs attention";
                view.Status.Message = inspection.Message;
                return;
            }
            string path = Path.Combine(evidence, "handoff.json");
            if (File.Exists(path))
                File.Copy(path, Path.Combine(evidence, "handoff-before-reconciliation-" + DateTime.UtcNow.ToString("yyyyMMddTHHmmssfff") + ".json"));
            HandoffRecord("handoff_reconciled", false, error: "Operator inspected the current session and acknowledged the unresolved attempt");
            view.InspectPrevious.Visibility = Visibility.Collapsed;
            view.AcknowledgePrevious.Visibility = Visibility.Collapsed;
            view.Handoff.IsEnabled = true;
            view.Status.Title = "Previous attempt acknowledged";
            view.Status.Message = inspection.Message + " A new handoff will still check RDP, desktop and Porthole before requesting elevation.";
            view.Status.Severity = InfoBarSeverity.Informational;
        } catch (Exception error) {
            view.Status.Title = "Could not acknowledge the previous attempt";
            view.Status.Message = error.Message;
            view.Status.Severity = InfoBarSeverity.Warning;
        }
    }

    async System.Threading.Tasks.Task ExecuteHandoffAsync(HandoffWindow view)
    {
        if (handoffCancellation != null || unresolved) return;
        using var cancellation = new CancellationTokenSource();
        handoffCancellation = cancellation;
        committed = false;
        workerPid = null;
        workerStarted = null;
        tray?.SetBusy(true);
        view.Handoff.IsEnabled = false; view.CancelAttempt.IsEnabled = true;
        view.CancelAttempt.Visibility = Visibility.Visible;
        HandoffContext? context = null;
        var info = view.Status;
        try {
            info.Title = "Checking desktop"; info.Message = "Checking the RDP session and Porthole."; info.Severity = InfoBarSeverity.Informational;
            context = await HandoffContext.DiscoverAsync(cancellation.Token);
            daemonPid = context.DaemonPid; daemonStarted = context.DaemonStarted;
            HandoffRecord("handoff_requesting_elevation", false);
            info.Title = "Waiting for Windows approval";
            string result = await WorkerChannel.HandoffAsync(async token => {
                HandoffRecord("handoff_armed", false);
                info.Title = "Preparing desktop handoff";
                await context.GrantAsync(view.Activate, WinRT.Interop.WindowNative.GetWindowHandle(view), token);
            }, worker => {
                workerPid = worker.Id;
                workerStarted = worker.StartTime.ToUniversalTime();
                // Persist before writing: even a partial send makes outcome uncertain.
                HandoffRecord("handoff_commit_pending", true);
                committed = true; view.CancelAttempt.IsEnabled = false;
                info.Title = "Disconnecting RDP";
            }, cancellation.Token);
            bool ready = await context.ObserveAsync(cancellation.Token);
            string status = result == "DONE" ? (ready ? "handoff_ready" : "handoff_needs_attention") : "handoff_outcome_unknown";
            HandoffRecord(status, result != "DONE", workerResult: result);
            info.Title = result == "DONE" ? (ready ? "Desktop available at console" : "Transferred; desktop needs attention") : "Handoff outcome needs inspection";
            info.Message = result == "DONE" ? "You remain signed in. Porthole and the agent keep their existing sessions." : "The worker did not confirm success. Inspect the session before trying again.";
            info.Severity = ready && result == "DONE" ? InfoBarSeverity.Success : InfoBarSeverity.Warning;
        } catch (Exception e) {
            bool cancelled = !committed && (e is OperationCanceledException || e is Win32Exception w && w.NativeErrorCode == 1223);
            HandoffRecord(committed ? "handoff_outcome_unknown" : cancelled ? "handoff_cancelled" : "handoff_not_committed", committed, error: e.Message);
            info.Title = committed ? "Handoff outcome needs inspection" : cancelled ? "Handoff cancelled" : "Handoff did not start";
            info.Message = committed ? "The connection was lost after commit. Inspect the Windows session; no automatic retry will occur." : e.Message;
            info.Severity = cancelled ? InfoBarSeverity.Informational : InfoBarSeverity.Warning;
        } finally {
            context?.Dispose(); handoffCancellation = null;
            tray?.SetBusy(false);
            view.Handoff.IsEnabled = !unresolved; view.CancelAttempt.IsEnabled = false;
            view.CancelAttempt.Visibility = Visibility.Collapsed;
            view.InspectPrevious.Visibility = unresolved ? Visibility.Visible : Visibility.Collapsed;
            view.AcknowledgePrevious.Visibility = Visibility.Collapsed;
        }
    }
}
