param(
    [Parameter(Mandatory)][ValidateSet('Start','Status','Approve','Cleanup')][string]$Action,
    [Parameter(Mandatory)][string]$Directory,
    [string]$Cli,
    [string]$Cleat,
    [string]$AgentCommand
)
$ErrorActionPreference = 'Stop'
$Directory = [IO.Path]::GetFullPath($Directory)
$statePath = Join-Path $Directory 'state.json'
$identityPath = Join-Path $Directory 'identity.json'
function Save-State { $script:state | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $statePath }
function Invoke-Cli([string[]]$Arguments) {
    $text = (& $script:state.Cli @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw $text }
    return $text
}
function Approve-Run {
    $old = $env:PORTHOLE_AGENT_TOKEN
    try {
        $env:PORTHOLE_AGENT_TOKEN = $null
        $requests = Invoke-Cli @('agents','requests','--json') | ConvertFrom-Json
        foreach ($request in $requests) {
            if ($request.agent_id -eq $script:identity.agent_id -and $request.status -eq 'pending') {
                Write-Output "Approving run request $($request.request_id): $($request.actions -join ', ')"
                Invoke-Cli @('agents','approve',$request.request_id,'--duration','persistent','--json') | Out-Null
            }
        }
    } finally { $env:PORTHOLE_AGENT_TOKEN = $old }
}
$oldToken = $env:PORTHOLE_AGENT_TOKEN
$oldRuntime = $env:CLEAT_RUNTIME_DIR
try {
    if ($Action -eq 'Start') {
        if (Test-Path -LiteralPath $Directory) { throw 'Choose a new run directory.' }
        foreach ($path in @($Cli,$Cleat,$AgentCommand)) {
            if (!$path -or !(Test-Path -LiteralPath $path)) { throw 'Start requires existing -Cli, -Cleat and -AgentCommand paths.' }
        }
        # Protect the directory before writing credentials. This is same-user
        # local-trust provisioning, not a separate operator security boundary.
        $acl = New-Object Security.AccessControl.DirectorySecurity
        $acl.SetAccessRuleProtection($true,$false)
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $rule = New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
        $acl.AddAccessRule($rule)
        [IO.Directory]::CreateDirectory($Directory,$acl) | Out-Null
        $state = [pscustomobject]@{ Cli=(Resolve-Path $Cli).Path; Cleat=(Resolve-Path $Cleat).Path; AgentCommand=(Resolve-Path $AgentCommand).Path; Runtime=(Join-Path $Directory 'runtime'); Server='workflow'; Session='agent'; Terminal=$null }
        Save-State
        $env:PORTHOLE_AGENT_TOKEN = $null
        $info = Invoke-Cli @('info')
        if ($info -notmatch 'system permission interactive_desktop: granted') { throw "BLOCKED: daemon lacks interactive desktop access: $info" }
        $helpText = & $state.Cleat launch --help
        if ($LASTEXITCODE -ne 0 -or "$helpText" -notmatch '--tag') { throw 'Cleat must support launch --tag.' }
        $identity = Invoke-Cli @('agents','create','--name','Windows workflow','--json') | ConvertFrom-Json
        $identity | ConvertTo-Json | Set-Content -LiteralPath $identityPath
        $env:PORTHOLE_AGENT_TOKEN = $identity.token
        $agentScript = Join-Path $PSScriptRoot 'agent.ps1'
        # Batch expansion makes these characters unsafe in literal paths.
        foreach ($path in @($Directory,$state.Cleat,$agentScript)) {
            if ($path -match '[%"\r\n]') { throw 'Batch launcher paths cannot contain percent, quotes or newlines.' }
        }
        $wrapper = Join-Path $Directory 'terminal.cmd'
        $agentLine = "powershell.exe -NoProfile -File `"$agentScript`" -Directory `"$Directory`""
        $agentWrapper = Join-Path $Directory 'agent.cmd'
        @('@echo off', $agentLine) | Set-Content -LiteralPath $agentWrapper -Encoding ASCII
        @('@echo off', "set `"CLEAT_RUNTIME_DIR=$($state.Runtime)`"",
          "`"$($state.Cleat)`" --server workflow launch agent --tag project=porthole --tag purpose=agent --cwd `"$Directory`" --cmd `"$agentWrapper`"",
          'if errorlevel 1 exit /b 1', ':wait', "if exist `"$(Join-Path $Directory 'stop-terminal')`" exit /b 0", 'timeout /t 1 /nobreak >nul', 'goto wait') | Set-Content -LiteralPath $wrapper -Encoding ASCII
        $launchArgs = @('launch','--app',"$env:SystemRoot\System32\conhost.exe",'--arg=-ForceNoHandoff','--arg=cmd.exe','--arg=/d','--arg=/c',"--arg=$wrapper",'--require-fresh-surface','--json')
        try { Invoke-Cli $launchArgs | Out-Null; throw 'New identity unexpectedly had launch permission.' }
        catch { if ($_.Exception.Message -notmatch 'agent_permission_needed') { throw } }
        Approve-Run
        $state.Terminal = Invoke-Cli $launchArgs | ConvertFrom-Json
        Save-State
    } else {
        $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        $identity = Get-Content -LiteralPath $identityPath -Raw | ConvertFrom-Json
        $env:PORTHOLE_AGENT_TOKEN = $identity.token
    }
    $env:CLEAT_RUNTIME_DIR = $state.Runtime
    switch ($Action) {
        'Approve' { Approve-Run }
        'Status' { & $state.Cleat --server $state.Server inspect --json $state.Session; if ($LASTEXITCODE) { throw 'Existing cleat session is unavailable.' } }
        'Cleanup' {
            if (!$state.Terminal.surface_id) { throw 'No terminal launch recorded; reconcile this run before cleanup.' }
            # The operator must close any run-owned app before ending its agent.
            & $state.Cleat --server $state.Server kill $state.Session
            if ($LASTEXITCODE) { throw 'Could not stop the owned cleat session.' }
            New-Item -ItemType File -Path (Join-Path $Directory 'stop-terminal') -Force | Out-Null
            $waitArgs = @('wait',$state.Terminal.surface_id,'--condition','gone')
            try { Invoke-Cli $waitArgs | Out-Null }
            catch {
                if ($_.Exception.Message -notmatch 'agent_permission_needed') { throw }
                Approve-Run
                Invoke-Cli $waitArgs | Out-Null
            }
            $env:PORTHOLE_AGENT_TOKEN = $null
            Invoke-Cli @('agents','revoke',$identity.agent_id,'--json') | Out-Null
            Remove-Item -LiteralPath $identityPath
            Write-Output 'Run identity revoked and terminal closed. Cleat recordings remain under runtime.'
        }
        'Start' {
            Write-Output "Run directory: $Directory"
            Write-Output "Attach from SSH with CLEAT_RUNTIME_DIR=$($state.Runtime), $($state.Cleat) --server $($state.Server) attach --no-create $($state.Session)"
        }
    }
} finally {
    $env:PORTHOLE_AGENT_TOKEN = $oldToken
    $env:CLEAT_RUNTIME_DIR = $oldRuntime
}
