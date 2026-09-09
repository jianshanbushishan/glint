param([string]$BinaryDirectory = "$PSScriptRoot/../target/debug")
$ErrorActionPreference = 'Stop'
$engineExe = (Resolve-Path "$BinaryDirectory/glint.exe").Path
$settingsExe = (Resolve-Path "$BinaryDirectory/glint-settings.exe").Path

# Release binaries use the Windows GUI subsystem. Explicit redirection and waiting
# make their CLI responses reliable in PowerShell as well as debug builds.
function Invoke-EngineCommand([string]$Command, [string]$ConfigDirectory) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new($engineExe)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in @($Command, '--config-dir', $ConfigDirectory)) {
        $startInfo.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::Start($startInfo)
    try {
        $outputTask = $process.StandardOutput.ReadToEndAsync()
        $errorTask = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(10000)) {
            $process.Kill()
            $process.WaitForExit()
            throw "Engine command timed out: $Command"
        }
        return @{ ExitCode = $process.ExitCode; Output = $outputTask.Result; Error = $errorTask.Result }
    } finally { $process.Dispose() }
}

# Real process regression checks, isolated from the user's config and mouse hooks.
foreach ($scenario in @('quit', 'crash', 'close-settings')) {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) "glint-lifecycle-$([guid]::NewGuid())"
    $engine = $null
    $settings = $null
    try {
        $initialized = Invoke-EngineCommand 'init' $testDir
        if ($initialized.ExitCode -ne 0) { throw 'Failed to initialize test config' }
        $engine = Start-Process $engineExe -ArgumentList @('--no-hooks', '--config-dir', "`"$testDir`"") -WindowStyle Hidden -PassThru
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            $status = Invoke-EngineCommand 'status' $testDir
            if ($status.ExitCode -eq 0) { break }
            if ($engine.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Engine failed to start' }
            Start-Sleep -Milliseconds 100
        } while ($true)
        if (($status.Output | ConvertFrom-Json).status.engine_pid -ne $engine.Id) { throw 'Wrong engine PID in status' }
        $settingsArgs = @('--config-dir', "`"$testDir`"")
        if ($scenario -eq 'close-settings') { $settingsArgs += '--smoke-test' }
        $settings = Start-Process $settingsExe -ArgumentList $settingsArgs -WindowStyle Hidden -PassThru
        # Allow the initial status/config requests to reach the UI timer.
        Start-Sleep -Seconds 2
        if ($settings.HasExited) { throw 'Settings exited while engine was running' }
        switch ($scenario) {
            'quit' {
                $quit = Invoke-EngineCommand 'quit' $testDir
                if ($quit.ExitCode -ne 0) { throw 'Quit command failed' }
            }
            'crash' { $engine.Kill() }
            # Smoke mode closes the actual GPUI window after 3s, exercising
            # on_window_closed rather than directly quitting the application.
            'close-settings' { }
        }
        if (!$settings.WaitForExit(5000)) { throw "$scenario left settings running" }
        if ($settings.ExitCode -ne 0) { throw "Settings failed: $($settings.ExitCode)" }
        if ($scenario -eq 'close-settings') {
            if ($engine.HasExited) { throw 'Closing settings also stopped the engine' }
        } elseif (!$engine.WaitForExit(5000)) { throw 'Engine failed to exit' }
        Write-Output "PASS: $scenario"
    } finally {
        # Only clean up processes created by this test; never find/kill by name.
        foreach ($child in @($settings, $engine)) {
            if ($null -ne $child) {
                if (!$child.HasExited) { $child.Kill(); $child.WaitForExit() }
                $child.Dispose()
            }
        }
    }
}
