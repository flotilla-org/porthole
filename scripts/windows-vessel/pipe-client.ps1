# JSON-only client for the existing HTTP-over-named-pipe API. Request bodies
# (including launch environment and tokens) are never printed or written to disk.
function Invoke-PortholeJson {
    param([string]$Method, [string]$Path, $Body = $null, [string]$Token = '', [string]$PipeName = "porthole-$env:USERNAME")
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', $PipeName, [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::Asynchronous)
    try {
        $pipe.Connect(3000)
        $payload = if ($null -eq $Body) { '' } else { $Body | ConvertTo-Json -Depth 15 -Compress }
        $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
        $header = "$Method $Path HTTP/1.1`r`nHost: localhost`r`nConnection: close`r`nContent-Length: $($bytes.Length)`r`n"
        if ($null -ne $Body) { $header += "Content-Type: application/json`r`n" }
        if ($Token) { $header += "Authorization: Bearer $Token`r`n" }
        $header += "`r`n"
        $prefix = [Text.Encoding]::ASCII.GetBytes($header)
        $pipe.Write($prefix, 0, $prefix.Length)
        $pipe.Write($bytes, 0, $bytes.Length)
        $pipe.Flush()
        $response = [IO.MemoryStream]::new()
        try {
            $copy = $pipe.CopyToAsync($response)
            if (-not $copy.Wait(30000)) { throw 'Porthole response timed out' }
            $raw = [Text.Encoding]::UTF8.GetString($response.ToArray())
        } finally { $response.Dispose() }
        $parts = $raw -split "`r`n`r`n", 2
        if ($parts.Count -ne 2 -or $parts[0] -notmatch '^HTTP/1\.[01] (\d{3})') { throw 'Invalid HTTP response' }
        $status = [int]$Matches[1]
        if ($parts[0] -match '(?im)^Transfer-Encoding:') { throw 'Expected bounded JSON response with content length' }
        $result = $null
        if ($parts[1].Trim()) {
            try { $result = $parts[1] | ConvertFrom-Json } catch {
                $message = $parts[1]
                if ($Token) { $message = $message.Replace($Token, '[redacted]') }
                $result = [pscustomobject]@{message=$message}
            }
        }
        return [pscustomobject]@{Status=$status; Body=$result}
    } finally { $pipe.Dispose() }
}
