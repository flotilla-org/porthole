using System;
using System.Diagnostics;

namespace Porthole.WindowsHelper;

internal enum HandoffSessionMode { Rdp, Console }

internal readonly record struct HandoffInspection(bool CanAcknowledge, string Message);

internal static class HandoffRecovery
{
    internal static HandoffInspection Assess(bool workerRunning, HandoffSessionMode? session)
    {
        if (workerRunning)
            return new(false, "A console handoff worker is still running. Wait for it to exit, then inspect again.");
        return session switch {
            HandoffSessionMode.Rdp => new(true, "No handoff worker is running. This session is active through RDP. The previous outcome remains unknown."),
            HandoffSessionMode.Console => new(true, "No handoff worker is running. This session is active at the console. The previous outcome remains unknown."),
            _ => new(false, "The current Windows session could not be confirmed active. Inspect again when the desktop is available."),
        };
    }

    internal static HandoffInspection InspectCurrent()
    {
        // A commit can outlive the helper that sent it. Re-arm only when no
        // worker from any helper instance can still transfer a session.
        foreach (var worker in Process.GetProcessesByName("PortholeConsoleWorker")) {
            using (worker) {
                if (!worker.HasExited) return Assess(true, null);
            }
        }
        return Assess(false, HandoffContext.ActiveSessionMode());
    }
}
