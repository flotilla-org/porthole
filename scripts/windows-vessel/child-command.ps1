function New-VesselChildCommand([string]$ScriptPath, [string]$RunDirectory) {
    # Cleat executes explicit commands through cmd.exe. Encode only the script
    # invocation so %, &, quotes and trailing slashes in paths stay literal
    # through both cmd.exe and Start-Process's string argument boundary.
    $scriptLiteral = $ScriptPath.Replace("'", "''")
    $directoryLiteral = $RunDirectory.Replace("'", "''")
    $invocation = "& '$scriptLiteral' -RunDirectory '$directoryLiteral'"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($invocation))
    return "powershell.exe -NoProfile -EncodedCommand $encoded"
}
