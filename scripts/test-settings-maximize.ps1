param([string]$BinaryDirectory = "$PSScriptRoot/../target/debug")
$ErrorActionPreference = 'Stop'
$settingsExe = (Resolve-Path "$BinaryDirectory/glint-settings.exe").Path
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SettingsWindowTest {
    private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    public static IntPtr FindWindow(int processId) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((hwnd, data) => {
            uint pid; GetWindowThreadProcessId(hwnd, out pid);
            if (pid == processId && (GetWindowLongW(hwnd, -16) & 0xC00000) == 0xC00000) {
                result = hwnd; return false;
            }
            return true;
        }, IntPtr.Zero);
        return result;
    }
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, UIntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr hwnd, int index);
}
'@
$testDir = Join-Path ([IO.Path]::GetTempPath()) ("glint-maximize-" + [guid]::NewGuid())
$process = Start-Process $settingsExe -ArgumentList @('--smoke-test', '--config-dir', "`"$testDir`"") -WindowStyle Hidden -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        Start-Sleep -Milliseconds 30
        $process.Refresh()
        $hwnd = [SettingsWindowTest]::FindWindow($process.Id)
    } while ($hwnd -eq [IntPtr]::Zero -and !$process.HasExited -and [DateTime]::UtcNow -lt $deadline)
    if ($hwnd -eq [IntPtr]::Zero) { throw 'Settings window not found' }
    # The HWND is enumerable before GPUI finishes the open-window callback that
    # installs the caption restrictions. Wait for that initialization to settle.
    do {
        $style = [SettingsWindowTest]::GetWindowLongW($hwnd, -16)
        if (!($style -band 0x10000)) { break }
        Start-Sleep -Milliseconds 30
        $process.Refresh()
    } while (!$process.HasExited -and [DateTime]::UtcNow -lt $deadline)
    $style = [SettingsWindowTest]::GetWindowLongW($hwnd, -16)
    if ($style -band 0x10000) { throw 'Maximize button is enabled' }
    if (!($style -band 0x40000)) { throw 'Window resizing was disabled' }
    foreach ($scenario in @('button', 'titlebar', 'command')) {
        switch ($scenario) {
            'button' {
                [void][SettingsWindowTest]::SendMessageW($hwnd, 0xA1, [UIntPtr]9, [IntPtr]::Zero)
                [void][SettingsWindowTest]::SendMessageW($hwnd, 0xA2, [UIntPtr]9, [IntPtr]::Zero)
            }
            'titlebar' { [void][SettingsWindowTest]::SendMessageW($hwnd, 0xA3, [UIntPtr]2, [IntPtr]::Zero) }
            'command' { [void][SettingsWindowTest]::SendMessageW($hwnd, 0x112, [UIntPtr]0xF030, [IntPtr]::Zero) }
        }
        Start-Sleep -Milliseconds 100
        if ([SettingsWindowTest]::IsZoomed($hwnd)) { throw "$scenario maximized the settings window" }
        Write-Output "PASS: $scenario cannot maximize settings"
    }
    [void][SettingsWindowTest]::SendMessageW($hwnd, 0x112, [UIntPtr]0xF020, [IntPtr]::Zero)
    if (![SettingsWindowTest]::IsIconic($hwnd)) { throw 'Minimize no longer works' }
    [void][SettingsWindowTest]::SendMessageW($hwnd, 0x112, [UIntPtr]0xF120, [IntPtr]::Zero)
    [void][SettingsWindowTest]::SendMessageW($hwnd, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
    if (!$process.WaitForExit(5000) -or $process.ExitCode -ne 0) { throw 'Settings did not close cleanly' }
    Write-Output 'PASS: minimize, restore, and close work'
} finally {
    if (!$process.HasExited) { $process.Kill(); $process.WaitForExit() }
    $process.Dispose()
}
