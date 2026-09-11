param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$outputDirectory = Join-Path $projectRoot 'dist\Glint'
Push-Location $projectRoot
try {
    $metadataJson = cargo metadata --no-deps --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Failed to read project version.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object { $_.name -eq 'glint' }).version
    if (-not $version) { throw 'Missing glint package version.' }
    if (-not $SkipBuild) {
        cargo build --workspace --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    foreach ($fileName in @('glint.exe', 'glint-settings.exe')) {
        $binaryPath = Join-Path $projectRoot "target\release\$fileName"
        if (-not (Test-Path -LiteralPath $binaryPath)) { throw "Missing $binaryPath" }
        Copy-Item -LiteralPath $binaryPath -Destination $outputDirectory -Force
    }
    foreach ($fileName in @('README.md', 'LICENSE')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $fileName) -Destination $outputDirectory -Force
    }
    Copy-Item -LiteralPath (Join-Path $projectRoot 'docs') -Destination $outputDirectory -Recurse -Force
    $archivePath = Join-Path $projectRoot "dist\Glint-v$version-windows-x64.zip"
    Compress-Archive -Path (Join-Path $outputDirectory '*') -DestinationPath $archivePath -Force
    Get-FileHash -LiteralPath $archivePath -Algorithm SHA256 | Format-List
    Write-Output "Ready: $outputDirectory"
}
finally { Pop-Location }
