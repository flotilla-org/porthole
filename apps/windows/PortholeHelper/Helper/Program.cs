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
    Window? window;
    TrayShell? tray;
    CancellationTokenSource? handoffCancellation;
    bool committed;
    bool unresolved;
    bool quitting;
    int? daemonPid;
    DateTime? daemonStarted;
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
    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var view = new HandoffWindow();
        window = view;
        const int width = 420;
        const int height = 275;
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
        if (unresolved) { info.Title = "Previous handoff needs inspection"; info.Message = "An earlier commit has an uncertain outcome. Inspect the Windows session before another attempt."; info.Severity = InfoBarSeverity.Warning; }
        void HandoffRecord(string status, bool pending, string? error = null, string? workerResult = null) {
            unresolved = pending;
            File.WriteAllText(Path.Combine(evidence, "handoff.json"), JsonSerializer.Serialize(new { status, unresolved = pending, error, worker_result = workerResult, utc = DateTime.UtcNow, daemon_pid = daemonPid, daemon_started = daemonStarted }));
            Record(status, error: error);
        }
        window.Closed += (_, _) => {
            handoffCancellation?.Cancel();
            tray?.Dispose();
            tray = null;
        };
        view.CancelAttempt.Click += (_, _) => handoffCancellation?.Cancel();
        view.Handoff.Click += async (_, _) => {
            if (handoffCancellation != null || unresolved) return;
            using var cancellation = new CancellationTokenSource();
            handoffCancellation = cancellation;
            committed = false;
            tray?.SetBusy(true);
            view.Handoff.IsEnabled = false; view.CancelAttempt.IsEnabled = true;
            view.CancelAttempt.Visibility = Visibility.Visible;
            HandoffContext? context = null;
            try {
                info.Title = "Checking desktop"; info.Message = "Checking the RDP session and Porthole."; info.Severity = InfoBarSeverity.Informational;
                context = await HandoffContext.DiscoverAsync(cancellation.Token);
                daemonPid = context.DaemonPid; daemonStarted = context.DaemonStarted;
                HandoffRecord("handoff_requesting_elevation", false);
                info.Title = "Waiting for Windows approval";
                string result = await WorkerChannel.HandoffAsync(async token => {
                    HandoffRecord("handoff_armed", false);
                    info.Title = "Preparing desktop handoff";
                    await context.GrantAsync(() => window.Activate(), WinRT.Interop.WindowNative.GetWindowHandle(window), token);
                }, () => {
                    committed = true; view.CancelAttempt.IsEnabled = false;
                    // Persist before writing: even a partial send makes outcome uncertain.
                    HandoffRecord("handoff_commit_pending", true);
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
            }
        };
        window.Activate(); window.AppWindow.Hide(); Record("ready");
    }
}
