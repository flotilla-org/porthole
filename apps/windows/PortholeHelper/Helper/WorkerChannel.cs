using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Win32.SafeHandles;

namespace Porthole.WindowsHelper;

internal static class WorkerChannel
{
    [StructLayout(LayoutKind.Sequential)]
    struct TokenLogonGroup
    {
        public uint Count;
        public IntPtr Sid;
        public uint Attributes;
    }

    internal static string ReadLogonSid(SafeAccessTokenHandle token)
    {
        // WindowsIdentity.Groups deliberately filters out SE_GROUP_LOGON_ID.
        // Query TokenLogonSid (28), which returns a TOKEN_GROUPS structure.
        if (GetTokenInformationBuffer(token, 28, IntPtr.Zero, 0, out int size))
            throw new InvalidOperationException("Unexpected empty logon information");
        int error = Marshal.GetLastWin32Error();
        if (error != 122) throw new Win32Exception(error);
        if (size < Marshal.SizeOf<TokenLogonGroup>() || size > 65536)
            throw new InvalidOperationException("Invalid logon information size");
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try {
            if (!GetTokenInformationBuffer(token, 28, buffer, size, out _)) throw new Win32Exception();
            var group = Marshal.PtrToStructure<TokenLogonGroup>(buffer);
            if (group.Count != 1 || group.Sid == IntPtr.Zero || (group.Attributes & 0xC0000000) != 0xC0000000)
                throw new InvalidOperationException("Exactly one logon SID required");
            return new SecurityIdentifier(group.Sid).Value;
        } finally { Marshal.FreeHGlobal(buffer); }
    }

    [DllImport("advapi32.dll", EntryPoint = "GetTokenInformation", SetLastError = true)]
    static extern bool GetTokenInformationBuffer(SafeAccessTokenHandle token, int information, IntPtr value, int size, out int returned);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafePipeHandle CreateFileW(string name, uint access, uint share, IntPtr attributes, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out uint pid);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool OpenProcessToken(SafeProcessHandle process, uint access, out SafeAccessTokenHandle token);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool GetTokenInformation(SafeAccessTokenHandle token, int information, out int value, int size, out int returned);

    static void Validate(Process worker, SafePipeHandle pipe)
    {
        // Retain the Process/SafeProcessHandle until the exchange is complete;
        // comparing a PID after releasing its handle would allow PID reuse.
        if (worker.HasExited || !GetNamedPipeServerProcessId(pipe, out uint pid) || pid != worker.Id)
            throw new InvalidOperationException("Unexpected worker pipe server");
        if (worker.SessionId != Process.GetCurrentProcess().SessionId)
            throw new InvalidOperationException("Worker session mismatch");
        if (!OpenProcessToken(worker.SafeHandle, 8, out var token)) throw new Win32Exception();
        using (token)
        using (var theirs = new WindowsIdentity(token.DangerousGetHandle()))
        using (var ours = WindowsIdentity.GetCurrent())
        {
            if (theirs.User != ours.User || ReadLogonSid(ours.AccessToken) != ReadLogonSid(token))
                throw new InvalidOperationException("Same-user, same-logon elevation required");
            if (!GetTokenInformation(token, 20, out int elevated, 4, out _) || elevated == 0)
                throw new InvalidOperationException("Worker is not elevated");
        }
    }

    static async Task Expect(NamedPipeClientStream pipe, string expected, CancellationToken cancellation)
    {
        byte[] message = new byte[4];
        await pipe.ReadExactlyAsync(message, cancellation);
        if (System.Text.Encoding.ASCII.GetString(message) != expected)
            throw new InvalidOperationException("Unexpected worker protocol message");
    }

    public static Task<string> HandoffAsync(Func<CancellationToken, Task> beforeCommit, Action committing, CancellationToken cancellation)
        => RunAsync(beforeCommit, committing, cancellation);

    static async Task<string> RunAsync(Func<CancellationToken, Task> beforeCommit, Action committing, CancellationToken callerCancellation)
    {
        var expectedRoot = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "PortholeHelper");
        if (!string.Equals(Path.TrimEndingDirectorySeparator(AppContext.BaseDirectory), expectedRoot, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Run the installed Porthole helper");
        var workerPath = Path.Combine(expectedRoot, "Worker", "PortholeConsoleWorker.exe");
        var nonce = Guid.NewGuid().ToString("N");
        var launch = new ProcessStartInfo(workerPath) {
            UseShellExecute = true, Verb = "runas", WindowStyle = ProcessWindowStyle.Hidden,
            WorkingDirectory = Path.GetDirectoryName(workerPath)!,
            Arguments = $"--console-handoff {Environment.ProcessId} {nonce}"
        };
        // ShellExecute/UAC can block; keep the WinUI dispatcher responsive.
        using var worker = await Task.Run(() => Process.Start(launch) ?? throw new InvalidOperationException("No worker handle"));
        _ = worker.SafeHandle;
        using var expiry = CancellationTokenSource.CreateLinkedTokenSource(callerCancellation);
        expiry.CancelAfter(TimeSpan.FromSeconds(30));
        var cancellation = expiry.Token;
        SafePipeHandle handle;
        while (true)
        {
            cancellation.ThrowIfCancellationRequested();
            if (worker.HasExited) throw new InvalidOperationException($"Worker rejected rendezvous (exit {worker.ExitCode})");
            // Exact rights omit FILE_CREATE_PIPE_INSTANCE; anonymous SQOS avoids
            // granting this server impersonation of the caller.
            handle = CreateFileW(@"\\.\pipe\Porthole.ConsoleWorker." + nonce, 0x0012019b, 0, IntPtr.Zero, 3, 0x40100000, IntPtr.Zero);
            if (!handle.IsInvalid) break;
            int error = Marshal.GetLastWin32Error();
            handle.Dispose();
            if (error != 2 && error != 231) throw new Win32Exception(error);
            await Task.Delay(25, cancellation);
        }
        using (handle)
        using (var pipe = new NamedPipeClientStream(PipeDirection.InOut, true, true, handle))
        {
            Validate(worker, handle);
            await Expect(pipe, "RDY2", cancellation);
            await beforeCommit(cancellation);
            Validate(worker, handle);
            cancellation.ThrowIfCancellationRequested();
            committing();
            await pipe.WriteAsync("CMT2"u8.ToArray(), cancellation);
            byte[] bytes = new byte[4];
            await pipe.ReadExactlyAsync(bytes, cancellation);
            string result = System.Text.Encoding.ASCII.GetString(bytes);
            if (result is not ("DONE" or "FAIL" or "UNKN")) throw new InvalidOperationException("Unexpected worker result");
            await pipe.WriteAsync("ACK1"u8.ToArray(), cancellation);
            await worker.WaitForExitAsync(cancellation);
            if (worker.ExitCode != 0) throw new InvalidOperationException($"Worker exited with code {worker.ExitCode}");
            return result;
        }
    }
}
