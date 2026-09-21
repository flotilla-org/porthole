param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
# THROWAWAY: real menu-to-daemon foreground grant experiment, not installed UI.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\scripts\windows-vessel\pipe-client.ps1')
$fixture = 'C:\dev\windows-parity-plan\agent-launch-target\debug\examples\desktop_fixture.exe'
$sessionId = (Get-Process -Id $PID).SessionId
[IO.Directory]::CreateDirectory($EvidenceDirectory) | Out-Null
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ConsoleDesktopProof {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError=true)] public static extern bool AllowSetForegroundWindow(uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, UIntPtr w, StringBuilder text, uint flags, uint timeout, out UIntPtr result);
    public static string ReadEditor(IntPtr hwnd) {
        var text = new StringBuilder(4096); UIntPtr result;
        if (SendMessageTimeout(GetDlgItem(hwnd, 1), 13, (UIntPtr)4096, text, 2, 1000, out result) == IntPtr.Zero)
            throw new Exception("Cannot read test editor");
        return text.ToString();
    }
}
'@
$identity = $null
$surface = $null
$ownedProcess = $null
$result = [ordered]@{status='starting'; session_id=$sessionId; started_utc=[DateTime]::UtcNow.ToString('o'); phases=@()}
function Save-State { $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'result.json') -Encoding UTF8 }
function Invoke-Owned([string]$Path, $Body) {
    $response = Invoke-PortholeJson POST $Path $Body $identity.token
    if ($response.Status -eq 403 -and $response.Body.code -eq 'agent_permission_needed') {
        $pending = Invoke-PortholeJson GET '/agent-permissions/requests'
        foreach ($request in @($pending.Body | Where-Object { $_.agent_id -eq $identity.agent_id -and $_.status -eq 'pending' })) {
            $launch = $request.description.operation.kind -eq 'launch' -and $request.description.operation.application -eq $fixture
            $editor = $request.description.surface.app_name -eq 'desktop_fixture.exe' -and $request.description.surface.title -eq 'Porthole #117 - test-owned editor'
            if (-not ($launch -or $editor)) { throw 'Unexpected permission request' }
            $approved = Invoke-PortholeJson POST "/agent-permissions/requests/$($request.request_id)/approve" @{duration=@{type='persistent'}; target=$request.target; actions=@($request.actions)}
            if ($approved.Status -ne 200) { throw 'Approval failed' }
        }
        $response = Invoke-PortholeJson POST $Path $Body $identity.token
    }
    if ($response.Status -ne 200) { throw "HTTP $($response.Status): $($response.Body.code): $($response.Body.message)" }
    return $response.Body
}
function Test-Desktop([string]$Phase, [string]$Expected) {
    $info = Invoke-PortholeJson GET '/info'
    $phaseResult = @{phase=$Phase; utc=[DateTime]::UtcNow.ToString('o'); session_listing=(@(& "$env:SystemRoot\System32\query.exe" session) -join "`n"); desktop=@($info.Body.adapters | ForEach-Object system_permissions); text_matches=$false}
    $result.phases += $phaseResult
    Save-State
    try {
        Invoke-Owned "/surfaces/$surface/focus" @{} | Out-Null
        $phaseResult.focus_ok = $true
        Invoke-Owned "/surfaces/$surface/text" @{text="$Phase;"} | Out-Null
        Start-Sleep -Milliseconds 300
        $phaseResult.text_matches = [ConsoleDesktopProof]::ReadEditor($ownedProcess.MainWindowHandle) -eq $Expected
        if (-not $phaseResult.text_matches) { throw "Editor text mismatch in $Phase" }
    } catch { $phaseResult.input_error = $_.Exception.Message }
    # Capture is an independent operation; record its result even if focus fails.
    try {
        $shot = Invoke-Owned "/surfaces/$surface/screenshot" @{}
        $png = Join-Path $EvidenceDirectory "$Phase.png"
        [IO.File]::WriteAllBytes($png, [Convert]::FromBase64String($shot.png_base64))
        $phaseResult.png = $png
        $phaseResult.sha256 = (Get-FileHash -LiteralPath $png).Hash
        $phaseResult.bounds = $shot.window_bounds
    } catch { $phaseResult.capture_error = $_.Exception.Message }
    Save-State
    if ($phaseResult.input_error -or $phaseResult.capture_error) { throw "Desktop probe failed in ${Phase}: input=$($phaseResult.input_error); capture=$($phaseResult.capture_error)" }
}
try {
    $created = Invoke-PortholeJson POST '/agent-identities' @{display_name='Temporary helper-owned foreground prototype'}
    if ($created.Status -notin 200,201) { throw 'Identity creation failed' }
    $identity = $created.Body
    $result.agent_id = $identity.agent_id
    $before = @(Get-Process desktop_fixture -ErrorAction SilentlyContinue | ForEach-Object Id)
    $launch = Invoke-Owned '/launches' @{kind=@{type='process'; app=$fixture; args=@()}; require_fresh_surface=$true; timeout_ms=5000}
    $surface = $launch.surface_id
    $new = @(Get-Process desktop_fixture | Where-Object { $_.Id -notin $before -and $_.Path -eq $fixture })
    if ($launch.confidence -ne 'strong' -or $new.Count -ne 1) { throw 'Fresh fixture identity not established' }
    $ownedProcess = $new[0]
    Test-Desktop 'before' 'before;'
    $daemon = Get-Process portholed
    if (@($daemon).Count -ne 1 -or $daemon.SessionId -ne $sessionId) { throw 'Expected one same-session Porthole daemon' }
    $result.daemon_pid = $daemon.Id
    $result.daemon_started = $daemon.StartTime.ToUniversalTime().ToString('o')
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $form = New-Object System.Windows.Forms.Form
    $form.Text = 'Porthole helper - foreground prototype'
    $form.ClientSize = New-Object System.Drawing.Size(520,170)
    $form.StartPosition = 'CenterScreen'
    $label = New-Object System.Windows.Forms.Label
    $label.Location = New-Object System.Drawing.Point(18,18)
    $label.Size = New-Object System.Drawing.Size(480,64)
    $label.Text = 'Open the helper menu, then choose Test foreground handoff. This will type only in the test editor. RDP stays connected.'
    $button = New-Object System.Windows.Forms.Button
    $button.Location = New-Object System.Drawing.Point(18,100)
    $button.Size = New-Object System.Drawing.Size(240,38)
    $button.Text = 'Open helper menu'
    $menu = New-Object System.Windows.Forms.ContextMenuStrip
    $item = $menu.Items.Add('Test foreground handoff')
    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = 150
    $button.Add_Click({ $menu.Show($button, (New-Object System.Drawing.Point(0,$button.Height))) })
    $item.Add_Click({
        $button.Enabled = $false
        $menu.Close()
        # Let the UI message loop finish closing our own menu before activation.
        $timer.Start()
    })
    $timer.Add_Tick({
        $timer.Stop()
        try {
            if ($menu.Visible) { throw 'Helper menu is still active' }
            $form.Activate()
            $result.helper_foreground = [ConsoleDesktopProof]::GetForegroundWindow() -eq $form.Handle
            if (-not $result.helper_foreground) { throw 'Helper does not own foreground; no grant attempted' }
            $live = Get-Process -Id $result.daemon_pid
            if ($live.StartTime.ToUniversalTime().ToString('o') -ne $result.daemon_started) { throw 'Daemon identity changed' }
            $result.grant_succeeded = [ConsoleDesktopProof]::AllowSetForegroundWindow([uint32]$live.Id)
            if (-not $result.grant_succeeded) { throw "Foreground grant failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
            Test-Desktop 'helper-grant' 'before;helper-grant;'
            $result.status = 'PASS'
        } catch { $result.status = 'FAIL'; $result.error = $_.Exception.Message }
        Save-State
        $form.Close()
    })
    $form.Controls.AddRange(@($label,$button))
    $result.status = 'waiting_for_helper_menu_click'
    Save-State
    [System.Windows.Forms.Application]::Run($form)
    $timer.Dispose()
    $menu.Dispose()
    $form.Dispose()
    if ($result.status -eq 'waiting_for_helper_menu_click') { $result.status = 'CANCELLED' }
} catch { $result.status = 'FAIL'; $result.error = $_.Exception.Message }
finally {
    $result.cleanup_errors = @()
    if ($surface) { try { Invoke-Owned "/surfaces/$surface/close" @{} | Out-Null } catch { $result.cleanup_errors += $_.Exception.Message } }
    if ($identity) {
        try { $revoked = Invoke-PortholeJson POST "/agent-identities/$($identity.agent_id)/revoke" @{}; if ($revoked.Status -ne 200) { throw 'Identity revocation failed' } } catch { $result.cleanup_errors += $_.Exception.Message }
    }
    $identity = $null
    $result.completed_utc = [DateTime]::UtcNow.ToString('o')
    Save-State
}
