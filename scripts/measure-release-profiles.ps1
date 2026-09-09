param(
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../.glint/profile-benchmark'),
    [int]$Rounds = 12
)
$ErrorActionPreference = 'Stop'
$rows = [System.Collections.Generic.List[object]]::new()
function Invoke-Sample([string]$Profile) {
    $binary = Join-Path $BinaryDirectory "$Profile.exe"
    $output = & $binary
    if ($LASTEXITCODE -ne 0) { throw "$Profile benchmark failed" }
    @($output | ForEach-Object { $_ | ConvertFrom-Json })
}
# Warm both binaries, then alternate which profile runs first in each pair.
foreach ($profile in @('thin', 'fat', 'fat', 'thin')) {
    $null = Invoke-Sample $profile
}
for ($round = 0; $round -lt $Rounds; $round++) {
    $order = if ($round % 2 -eq 0) { @('thin', 'fat') } else { @('fat', 'thin') }
    foreach ($profile in $order) {
        foreach ($sample in (Invoke-Sample $profile)) {
            $sample | Add-Member -NotePropertyName profile -NotePropertyValue $profile
            $sample | Add-Member -NotePropertyName round -NotePropertyValue $round
            $rows.Add($sample)
        }
    }
    Write-Host "Completed pair $($round + 1)/$Rounds"
}
$rows | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $BinaryDirectory 'samples.json')
foreach ($dense in @($false, $true)) {
    foreach ($metric in @('recognition_ms', 'prefix_ms', 'build_ms')) {
        $medians = @{}
        foreach ($profile in @('thin', 'fat')) {
            $values = @($rows | Where-Object { $_.dense -eq $dense -and $_.profile -eq $profile } |
                ForEach-Object { $_.$metric } | Sort-Object)
            $mid = [int][math]::Floor($values.Count / 2)
            $median = if ($values.Count % 2) { $values[$mid] } else { ($values[$mid - 1] + $values[$mid]) / 2 }
            $medians[$profile] = $median
            Write-Host "$dense $metric $profile median=$([math]::Round($median, 3)) range=$([math]::Round($values[0], 3))..$([math]::Round($values[-1], 3))"
        }
        Write-Host "time reduction: $([math]::Round((1 - $medians['fat'] / $medians['thin']) * 100, 2))%"
    }
}
