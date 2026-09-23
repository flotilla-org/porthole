using System;
using System.Linq;
using System.Security.Principal;
using Microsoft.Win32.SafeHandles;
using Porthole.WindowsHelper;

using var identity = WindowsIdentity.GetCurrent();
if (identity.Groups!.Any(g => g.Value.StartsWith("S-1-5-5-", StringComparison.Ordinal)))
    throw new Exception("Expected .NET's group list to exclude logon SIDs");
var logon = WorkerChannel.ReadLogonSid(identity.AccessToken);
if (!logon.StartsWith("S-1-5-5-", StringComparison.Ordinal))
    throw new Exception("Expected an actual Windows logon SID");
using (var duplicate = new WindowsIdentity(identity.AccessToken.DangerousGetHandle())) {
    if (duplicate.User != identity.User) throw new Exception("Duplicated token changed its user");
}
if (WorkerChannel.ReadLogonSid(identity.AccessToken) != logon)
    throw new Exception("Disposing WindowsIdentity closed the original token");
using var second = WindowsIdentity.GetCurrent();
if (WorkerChannel.ReadLogonSid(second.AccessToken) != logon)
    throw new Exception("Same-process token identities must match");
using var invalid = new SafeAccessTokenHandle(IntPtr.Zero);
try {
    WorkerChannel.ReadLogonSid(invalid);
    throw new Exception("Invalid token accepted");
} catch (System.ComponentModel.Win32Exception) { }
Console.WriteLine("PASS: logon SID query, token duplication, repeat identity, invalid-token rejection");
if (HandoffRecovery.Assess(true, HandoffSessionMode.Rdp).CanAcknowledge
    || HandoffRecovery.Assess(false, null).CanAcknowledge
    || !HandoffRecovery.Assess(false, HandoffSessionMode.Rdp).CanAcknowledge
    || !HandoffRecovery.Assess(false, HandoffSessionMode.Console).CanAcknowledge)
    throw new Exception("Unresolved handoff was re-armed without an inactive worker and active session");
Console.WriteLine("PASS: unresolved handoff requires worker exit, active session and explicit acknowledgement");
if (args.Contains("--check-desktop")) {
    using var timeout = new System.Threading.CancellationTokenSource(TimeSpan.FromSeconds(10));
    using var context = await HandoffContext.DiscoverAsync(timeout.Token);
    if (!await context.DesktopReadyAsync(timeout.Token)) throw new Exception("Desktop readiness changed");
    Console.WriteLine($"PASS: active RDP, retained daemon {context.DaemonPid}, authenticated API pipe, interactive desktop");
}
