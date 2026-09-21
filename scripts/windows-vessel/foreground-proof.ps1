param(
    [Parameter(Mandatory=$true)][string]$RunDirectory,
    [Parameter(Mandatory=$true)][string]$EvidencePath,
    [string]$PipeName = "porthole-$env:USERNAME",
    [switch]$GrantFromCaller,
    [switch]$InterveningInput,
    [switch]$SeparateInputProcess,
    [switch]$InputAssist,
    [ValidateRange(1,100)][int]$Iterations = 6,
    [switch]$InputWorker,
    [long]$InputHwnd
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'pipe-client.ps1')
$config = Get-Content (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
$state = Get-Content (Join-Path $RunDirectory 'state.json') -Raw | ConvertFrom-Json
$daemon = Get-Process -Id $state.portholed_pid
if ($daemon.StartTime.ToUniversalTime().ToString('o') -ne $state.portholed_started) { throw 'Daemon identity changed' }
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ForegroundProof {
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x, y; }
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
    public static long Cursor() { POINT point; if (!GetCursorPos(out point)) throw new Exception("Cannot read cursor"); return ((long)point.x << 32) | (uint)point.y; }
    public static int Modifiers() {
        int mask = 0; int[] keys = { 16, 17, 18, 91, 92, 1, 2, 4 };
        for (int i = 0; i < keys.Length; i++) if (GetAsyncKeyState(keys[i]) < 0) mask |= 1 << i;
        return mask;
    }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size=32)] struct UNION { [FieldOffset(0)] public KEYBDINPUT key; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION data; }
    [DllImport("user32.dll")] static extern uint SendInput(uint count, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    public static void SendTestKey() {
        var inputs = new INPUT[2];
        inputs[0].type = inputs[1].type = 1;
        inputs[0].data.key.vk = inputs[1].data.key.vk = 0x87; // F24, no editor text
        inputs[1].data.key.flags = 2;
        if (SendInput(2, inputs, Marshal.SizeOf(typeof(INPUT))) != 2) throw new Exception("Test input failed");
    }
    public static void SendEmptyMouse() {
        if (SendInput(1, new INPUT[1], Marshal.SizeOf(typeof(INPUT))) != 1) throw new Exception("Empty mouse input failed");
    }
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool AllowSetForegroundWindow(uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, UIntPtr w, StringBuilder text, uint flags, uint timeout, out UIntPtr result);
    public static string ReadEditor(IntPtr hwnd) {
        var text = new StringBuilder(4096); UIntPtr result;
        if (SendMessageTimeout(GetDlgItem(hwnd, 1), 13, (UIntPtr)4096, text, 2, 1000, out result) == IntPtr.Zero)
            throw new Exception("Cannot read test editor");
        return text.ToString();
    }
}
'@
if ($InputWorker) {
    if ([ForegroundProof]::GetForegroundWindow() -ne [IntPtr]$InputHwnd) { throw 'Test fixture lost foreground before worker input' }
    [ForegroundProof]::SendTestKey()
    exit 0
}
function Invoke-ProofJson([string]$Method, [string]$Path, $Body=$null, [string]$Token='') {
    Invoke-PortholeJson $Method $Path $Body $Token -PipeName $PipeName
}
$identity = $null
$windows = @()
$steps = @()
$failure = $null
function Invoke-Owned([string]$Path, $Body) {
    $response = Invoke-ProofJson POST $Path $Body $identity.token
    if ($response.Status -eq 403 -and $response.Body.code -eq 'agent_permission_needed') {
        $pending = Invoke-ProofJson GET '/agent-permissions/requests'
        foreach ($request in @($pending.Body | Where-Object { $_.agent_id -eq $identity.agent_id -and $_.status -eq 'pending' })) {
            $launch = $request.description.operation.kind -eq 'launch' -and $request.description.operation.application -eq $config.fixture
            $fixture = $request.description.surface.app_name -eq 'desktop_fixture.exe' -and $request.description.surface.title -eq 'Porthole #117 - test-owned editor'
            if (-not ($launch -or $fixture)) { throw 'Unexpected permission request' }
            $approved = Invoke-ProofJson POST "/agent-permissions/requests/$($request.request_id)/approve" @{duration=@{type='persistent'}; target=$request.target; actions=@($request.actions)}
            if ($approved.Status -ne 200) { throw 'Approval failed' }
        }
        $response = Invoke-ProofJson POST $Path $Body $identity.token
    }
    return $response
}
try {
    $created = Invoke-ProofJson POST '/agent-identities' @{display_name='Temporary two-window foreground proof'}
    if ($created.Status -ne 200 -and $created.Status -ne 201) { throw 'Identity creation failed' }
    $identity = $created.Body
    foreach ($index in 0..1) {
        $before = @(Get-Process desktop_fixture -ErrorAction SilentlyContinue | ForEach-Object Id)
        $launch = Invoke-Owned '/launches' @{kind=@{type='process'; app=$config.fixture; args=@()}; require_fresh_surface=$true; timeout_ms=5000}
        if ($launch.Status -ne 200 -or $launch.Body.confidence -ne 'strong') { throw 'Fresh fixture launch failed' }
        $window = @{surface=$launch.Body.surface_id; process=$null; hwnd=[IntPtr]::Zero; expected=''}
        $windows += $window
        $owned = @(Get-Process desktop_fixture | Where-Object { $_.Id -notin $before -and $_.Path -eq $config.fixture })
        if ($owned.Count -ne 1) { throw 'Expected exactly one new fixture process' }
        $window.process = $owned[0]
        $window.hwnd = $owned[0].MainWindowHandle
    }
    foreach ($step in 0..($Iterations - 1)) {
        $index = $step % 2
        $target = $windows[$index]
        if ($InterveningInput) {
            $other = $windows[1 - $index]
            # Test setup only: establish the other fixture as the input recipient.
            # The separate worker below then removes this caller's eligibility.
            if ($SeparateInputProcess) { [ForegroundProof]::SendEmptyMouse() }
            [ForegroundProof]::SetForegroundWindow($other.hwnd) | Out-Null
            if ([ForegroundProof]::GetForegroundWindow() -ne $other.hwnd) { throw 'Cannot establish intervening-input fixture foreground' }
            if ($SeparateInputProcess) {
                & powershell.exe -NoProfile -File $PSCommandPath -RunDirectory $RunDirectory -EvidencePath $EvidencePath -InputWorker -InputHwnd $other.hwnd.ToInt64()
                if ($LASTEXITCODE -ne 0) { throw 'Intervening input worker failed' }
            } else { [ForegroundProof]::SendTestKey() }
        }
        $before = [ForegroundProof]::GetForegroundWindow()
        $cursorBefore = [ForegroundProof]::Cursor()
        $modifiersBefore = [ForegroundProof]::Modifiers()
        $grant = $null
        if ($InputAssist) { [ForegroundProof]::SendEmptyMouse() }
        if ($GrantFromCaller) { $grant = [ForegroundProof]::AllowSetForegroundWindow($state.portholed_pid) }
        $focused = Invoke-Owned "/surfaces/$($target.surface)/focus" @{}
        $after = [ForegroundProof]::GetForegroundWindow()
        $cursorUnchanged = [ForegroundProof]::Cursor() -eq $cursorBefore
        $modifiersUnchanged = [ForegroundProof]::Modifiers() -eq $modifiersBefore
        $inputStatus = $null
        if ($focused.Status -eq 200 -and $after -eq $target.hwnd) {
            $marker = "window$index-step$step;"
            $typed = Invoke-Owned "/surfaces/$($target.surface)/text" @{text=$marker}
            $inputStatus = $typed.Status
            if ($typed.Status -eq 200) { $target.expected += $marker }
        }
        $deadline = [DateTime]::UtcNow.AddSeconds(1)
        do {
            $matches = @($windows | ForEach-Object { [ForegroundProof]::ReadEditor($_.hwnd) -eq $_.expected })
            if ($false -notin $matches) { break }
            Start-Sleep -Milliseconds 10
        } while ([DateTime]::UtcNow -lt $deadline)
        $steps += @{step=$step; target=$index; grant=$grant; focus_status=$focused.Status; focus_error=$focused.Body; foreground_matches=($after -eq $target.hwnd); switched=($before -ne $after); cursor_unchanged=$cursorUnchanged; modifiers_unchanged=$modifiersUnchanged; input_status=$inputStatus; text_matches=$matches}
    }
} catch { $failure = $_.Exception.Message } finally {
    foreach ($window in $windows) {
        try { $closed = Invoke-Owned "/surfaces/$($window.surface)/close" @{}; if ($closed.Status -ne 200) { throw 'Close failed' } } catch { $failure = "Cleanup failed: $($_.Exception.Message)" }
    }
    if ($identity) {
        try {
            $revoked = Invoke-ProofJson POST "/agent-identities/$($identity.agent_id)/revoke" @{}
            if ($revoked.Status -ne 200) { throw 'Identity revocation failed' }
        } catch { $failure = "Cleanup failed: $($_.Exception.Message)" }
    }
}
$passed = -not $failure -and $steps.Count -eq $Iterations -and @($steps | Where-Object { $_.focus_status -ne 200 -or $_.input_status -ne 200 -or -not $_.foreground_matches -or -not $_.cursor_unchanged -or -not $_.modifiers_unchanged -or $false -in $_.text_matches }).Count -eq 0 -and @($steps | Where-Object switched).Count -ge ($Iterations - 1)
@{passed=$passed; caller_pid=$PID; grant_from_caller=[bool]$GrantFromCaller; input_assist=[bool]$InputAssist; intervening_input=[bool]$InterveningInput; separate_input_process=[bool]$SeparateInputProcess; steps=$steps; failure=$failure; utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 $EvidencePath
Write-Output "Foreground switching passed: $passed; evidence: $EvidencePath"
if (-not $passed) { exit 1 }
