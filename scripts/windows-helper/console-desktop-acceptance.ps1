param(
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [Parameter(Mandatory=$true)][string]$AfterHandoffUtc
)

# Throwaway native acceptance for the installed helper's console handoff.
# The temporary agent token stays in this process and is never serialized.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\windows-vessel\pipe-client.ps1')
$fixture = 'C:\dev\windows-parity-plan\agent-launch-target\debug\examples\desktop_fixture.exe'
$handoffFile = Join-Path $env:LOCALAPPDATA 'Porthole\helper\handoff.json'
$sessionId = (Get-Process -Id $PID).SessionId
$after = [DateTime]::Parse($AfterHandoffUtc).ToUniversalTime()
[IO.Directory]::CreateDirectory($EvidenceDirectory) | Out-Null

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class TestEditorText {
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, UIntPtr w, StringBuilder text, uint flags, uint timeout, out UIntPtr result);
    public static string Read(IntPtr hwnd) {
        var text = new StringBuilder(4096); UIntPtr result;
        if (SendMessageTimeout(GetDlgItem(hwnd, 1), 13, (UIntPtr)4096, text, 2, 1000, out result) == IntPtr.Zero)
            throw new Exception("Cannot read test editor");
        return text.ToString();
    }
}
'@

$state = [ordered]@{
    status = 'starting'
    started_utc = [DateTime]::UtcNow.ToString('o')
    session_id = $sessionId
    after_handoff_utc = $after.ToString('o')
    phases = @()
}
$identity = $null
$surface = $null
$ownedProcess = $null
function Save-State {
    $state | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'result.json') -Encoding UTF8
}
function Invoke-Owned([string]$Path, $Body) {
    $response = Invoke-PortholeJson POST $Path $Body $identity.token
    if ($response.Status -eq 403 -and $response.Body.code -eq 'agent_permission_needed') {
        $pending = Invoke-PortholeJson GET '/agent-permissions/requests'
        foreach ($request in @($pending.Body | Where-Object { $_.agent_id -eq $identity.agent_id -and $_.status -eq 'pending' })) {
            $launch = $request.description.operation.kind -eq 'launch' -and $request.description.operation.application -eq $fixture
            $editor = $request.description.surface.app_name -eq 'desktop_fixture.exe' -and $request.description.surface.title -eq 'Porthole #117 - test-owned editor'
            if (-not ($launch -or $editor)) { throw 'Unexpected permission request' }
            $approved = Invoke-PortholeJson POST "/agent-permissions/requests/$($request.request_id)/approve" @{duration=@{type='persistent'}; target=$request.target; actions=@($request.actions)}
            if ($approved.Status -ne 200) { throw 'Fixture approval failed' }
        }
        $response = Invoke-PortholeJson POST $Path $Body $identity.token
    }
    if ($response.Status -ne 200) { throw "HTTP $($response.Status): $($response.Body.code): $($response.Body.message)" }
    return $response.Body
}
function Probe-Desktop([string]$Phase, [string]$Expected) {
    $info = Invoke-PortholeJson GET '/info'
    $entry = [ordered]@{
        phase = $Phase
        utc = [DateTime]::UtcNow.ToString('o')
        session_listing = (@(& "$env:SystemRoot\System32\query.exe" session) -join "`n")
        desktop = @($info.Body.adapters | ForEach-Object system_permissions)
        focus_ok = $false
        text_matches = $false
        capture_ok = $false
    }
    $state.phases += $entry
    Save-State
    try {
        Invoke-Owned "/surfaces/$surface/focus" @{} | Out-Null
        $entry.focus_ok = $true
        Invoke-Owned "/surfaces/$surface/text" @{text="$Phase;"} | Out-Null
        # SendInput queues events; let the test editor consume them before
        # judging delivery, without retrying text whose outcome is uncertain.
        for ($attempt = 0; $attempt -lt 20; $attempt++) {
            Start-Sleep -Milliseconds 100
            $entry.observed_text = [TestEditorText]::Read($ownedProcess.MainWindowHandle)
            if ($entry.observed_text -eq $Expected) { $entry.text_matches = $true; break }
        }
        if (-not $entry.text_matches) { throw 'Test editor text mismatch' }
    } catch { $entry.input_error = $_.Exception.Message }
    try {
        $shot = Invoke-Owned "/surfaces/$surface/screenshot" @{}
        $png = Join-Path $EvidenceDirectory "$Phase.png"
        [IO.File]::WriteAllBytes($png, [Convert]::FromBase64String($shot.png_base64))
        $entry.png = $png
        $entry.sha256 = (Get-FileHash -LiteralPath $png).Hash
        $entry.bounds = $shot.window_bounds
        $entry.capture_ok = $true
    } catch { $entry.capture_error = $_.Exception.Message }
    Save-State
    if (-not ($entry.focus_ok -and $entry.text_matches -and $entry.capture_ok)) { throw "Desktop probe failed in $Phase" }
}

try {
    if ($sessionId -ne 1) { throw "Unexpected observer session $sessionId" }
    $created = Invoke-PortholeJson POST '/agent-identities' @{display_name='Temporary installed-helper console acceptance'}
    if ($created.Status -notin 200,201) { throw 'Identity creation failed' }
    $identity = $created.Body
    $state.agent_id = $identity.agent_id
    $before = @(Get-Process desktop_fixture -ErrorAction SilentlyContinue | ForEach-Object Id)
    $launch = Invoke-Owned '/launches' @{kind=@{type='process'; app=$fixture; args=@()}; require_fresh_surface=$true; timeout_ms=5000}
    $surface = $launch.surface_id
    $new = @(Get-Process desktop_fixture | Where-Object { $_.Id -notin $before -and $_.Path -eq $fixture })
    if ($launch.confidence -ne 'strong' -or $new.Count -ne 1) { throw 'Fresh fixture identity not established' }
    $ownedProcess = $new[0]
    $state.fixture_pid = $ownedProcess.Id
    $state.surface_id = $surface
    Probe-Desktop 'rdp' 'rdp;'
    $state.status = 'armed'
    Save-State
    $deadline = [DateTime]::UtcNow.AddMinutes(15)
    do {
        Start-Sleep -Milliseconds 250
        if (-not (Test-Path -LiteralPath $handoffFile)) { continue }
        try { $handoff = Get-Content -LiteralPath $handoffFile -Raw | ConvertFrom-Json } catch { continue }
        if ($handoff.status -ne 'handoff_ready' -or [DateTime]::Parse($handoff.utc).ToUniversalTime() -le $after) { continue }
        $listing = @(& "$env:SystemRoot\System32\query.exe" session)
        if (@($listing | Where-Object { $_ -match "^>?\s*console\s+\S+\s+$sessionId\s+Active\b" }).Count -eq 1) { break }
    } while ([DateTime]::UtcNow -lt $deadline)
    if ([DateTime]::UtcNow -ge $deadline) { throw 'Timed out waiting for a new active console handoff' }
    $state.handoff_utc = $handoff.utc
    $state.status = 'testing_console'
    Save-State
    Start-Sleep -Seconds 3
    Probe-Desktop 'console' 'rdp;console;'
    $state.status = 'PASS'
} catch {
    $state.status = 'FAIL'
    $state.error = $_.Exception.Message
} finally {
    $state.cleanup_errors = @()
    if ($surface) {
        try { Invoke-Owned "/surfaces/$surface/close" @{} | Out-Null }
        catch { $state.cleanup_errors += $_.Exception.Message }
    }
    if ($identity) {
        try {
            $revoked = Invoke-PortholeJson POST "/agent-identities/$($identity.agent_id)/revoke" @{}
            if ($revoked.Status -ne 200) { throw 'Identity revocation failed' }
        } catch { $state.cleanup_errors += $_.Exception.Message }
    }
    $identity = $null
    $state.completed_utc = [DateTime]::UtcNow.ToString('o')
    Save-State
}
