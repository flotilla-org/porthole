using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Linq;
using System.Net.Http;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Win32.SafeHandles;

namespace Porthole.WindowsHelper;

internal sealed class HandoffContext : IDisposable
{
    [DllImport("wtsapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool WTSQuerySessionInformationW(IntPtr server, int session, int info, out IntPtr buffer, out int size);
    [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr buffer);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out uint pid);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool OpenProcessToken(SafeProcessHandle process, uint access, out SafeAccessTokenHandle token);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError = true)] static extern bool AllowSetForegroundWindow(uint pid);

    readonly Process daemon;
    readonly DateTime started;
    readonly string executable;
    readonly int session = Process.GetCurrentProcess().SessionId;
    public int DaemonPid => daemon.Id;
    public DateTime DaemonStarted => started;

    HandoffContext(Process daemon, DateTime started, string executable)
    { this.daemon = daemon; this.started = started; this.executable = executable; }

    static int Query(int session, int info, bool shortValue = false)
    {
        if (!WTSQuerySessionInformationW(IntPtr.Zero, session, info, out var data, out int size)) throw new Win32Exception();
        try {
            if (size < (shortValue ? 2 : 4)) throw new InvalidOperationException("Invalid Windows session response");
            return shortValue ? Marshal.ReadInt16(data) : Marshal.ReadInt32(data);
        } finally { WTSFreeMemory(data); }
    }

    internal static void RequireActiveRdp()
    {
        int session = Process.GetCurrentProcess().SessionId;
        if (session == 0 || Query(session, 8) != 0) throw new InvalidOperationException("An active interactive session is required");
        if (Query(session, 16, true) != 2) throw new InvalidOperationException("This session is already at the console or is not connected through RDP");
    }

    void ValidateDaemon()
    {
        if (daemon.HasExited || daemon.SessionId != session || daemon.StartTime.ToUniversalTime() != started
            || !string.Equals(daemon.MainModule?.FileName, executable, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Porthole process identity changed");
        if (!OpenProcessToken(daemon.SafeHandle, 8, out var token)) throw new Win32Exception();
        using (token)
        using (var theirs = new WindowsIdentity(token.DangerousGetHandle()))
        using (var ours = WindowsIdentity.GetCurrent()) {
            if (theirs.User != ours.User || WorkerChannel.ReadLogonSid(token) != WorkerChannel.ReadLogonSid(ours.AccessToken))
                throw new InvalidOperationException("Porthole must belong to this user and logon session");
        }
    }

    public static async Task<HandoffContext> DiscoverAsync(CancellationToken cancellation)
    {
        RequireActiveRdp();
        // Existing startup state is a discovery hint, then checked against the
        // retained process and the kernel PID of the actual API pipe server.
        var path = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Porthole", "startup", "startup.json");
        using var state = JsonDocument.Parse(await File.ReadAllTextAsync(path, cancellation));
        var root = state.RootElement;
        string executable = root.GetProperty("executable").GetString()!;
        if (!string.Equals(Path.GetFileName(executable), "portholed.exe", StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Unexpected daemon executable");
        var process = Process.GetProcessById(root.GetProperty("portholed_pid").GetInt32());
        var context = new HandoffContext(process, root.GetProperty("portholed_started").GetDateTime().ToUniversalTime(), executable);
        try {
            _ = process.SafeHandle;
            if (!await context.DesktopReadyAsync(cancellation)) throw new InvalidOperationException("Porthole reports that the desktop is unavailable");
            return context;
        } catch { context.Dispose(); throw; }
    }

    public async Task<bool> DesktopReadyAsync(CancellationToken cancellation)
    {
        ValidateDaemon();
        using var handler = new SocketsHttpHandler { UseProxy = false };
        handler.ConnectCallback = async (_, token) => {
            var pipe = new NamedPipeClientStream(".", "porthole-" + Environment.UserName, PipeDirection.InOut, PipeOptions.Asynchronous,
                TokenImpersonationLevel.Anonymous);
            try {
                await pipe.ConnectAsync(token);
                if (!GetNamedPipeServerProcessId(pipe.SafePipeHandle, out uint pid) || pid != daemon.Id)
                    throw new InvalidOperationException("Unexpected Porthole pipe server");
                ValidateDaemon();
                return pipe;
            } catch { pipe.Dispose(); throw; }
        };
        using var client = new HttpClient(handler) { Timeout = TimeSpan.FromSeconds(3), MaxResponseContentBufferSize = 65536 };
        using var response = await client.GetAsync("http://localhost/info", cancellation);
        response.EnsureSuccessStatusCode();
        using var info = JsonDocument.Parse(await response.Content.ReadAsStringAsync(cancellation));
        return info.RootElement.GetProperty("adapters").EnumerateArray().Any(a => a.GetProperty("name").GetString() == "windows"
            && a.GetProperty("system_permissions").EnumerateArray().Any(p => p.GetProperty("name").GetString() == "interactive_desktop" && p.GetProperty("granted").GetBoolean()));
    }

    public async Task GrantAsync(Action activate, IntPtr window, CancellationToken cancellation)
    {
        RequireActiveRdp();
        if (!await DesktopReadyAsync(cancellation)) throw new InvalidOperationException("Desktop became unavailable");
        activate();
        for (int i = 0; i < 10 && GetForegroundWindow() != window; i++) await Task.Delay(50, cancellation);
        ValidateDaemon();
        RequireActiveRdp();
        if (GetForegroundWindow() != window || !AllowSetForegroundWindow((uint)daemon.Id))
            throw new InvalidOperationException("Windows denied foreground activation; no handoff was committed");
    }

    public async Task<bool> ObserveAsync(CancellationToken cancellation)
    {
        var elapsed = Stopwatch.StartNew();
        while (elapsed.Elapsed < TimeSpan.FromSeconds(10)) {
            cancellation.ThrowIfCancellationRequested();
            try {
                if (Query(session, 8) == 0 && Query(session, 16, true) == 0 && await DesktopReadyAsync(cancellation)) return true;
            } catch (Exception) when (!cancellation.IsCancellationRequested) { }
            await Task.Delay(500, cancellation);
        }
        return false;
    }

    public void Dispose() => daemon.Dispose();
}
