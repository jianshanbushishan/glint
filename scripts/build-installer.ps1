[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$IsccPath,
    [string]$RuntimeDirectory
)

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

# Accept an explicit compiler, PATH, a normal install, or a local tools cache.
if (-not $IsccPath) {
    $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($compiler) { $IsccPath = $compiler.Source }
    else {
        $candidates = @(
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
            (Join-Path $projectRoot 'target\installer-tools\inno\{app}\ISCC.exe')
        )
        $IsccPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath -PathType Leaf)) {
    throw 'Inno Setup 6 compiler not found. Install from https://jrsoftware.org/isdl.php or pass -IsccPath C:\path\ISCC.exe.'
}
$IsccPath = (Resolve-Path -LiteralPath $IsccPath).Path

# Use redistributable files from Visual Studio, never DLLs from System32.
if (-not $RuntimeDirectory) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path -LiteralPath $vswhere) {
        $vsRoot = & $vswhere -latest -products '*' -property installationPath
        if ($vsRoot) {
            $RuntimeDirectory = Get-ChildItem -Path "$vsRoot\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT" -Directory |
                Sort-Object { [version]$_.Parent.Parent.Name } -Descending |
                Select-Object -First 1 -ExpandProperty FullName
        }
    }
}
if (-not $RuntimeDirectory -or -not (Test-Path -LiteralPath (Join-Path $RuntimeDirectory 'vcruntime140.dll'))) {
    throw 'Visual C++ x64 redistributable CRT directory not found. Pass -RuntimeDirectory pointing to VC\Redist\MSVC\<version>\x64\Microsoft.VC143.CRT.'
}
$RuntimeDirectory = (Resolve-Path -LiteralPath $RuntimeDirectory).Path

Push-Location $projectRoot
try {
    $metadataJson = cargo metadata --no-deps --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Failed to read Cargo metadata.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'glint').version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "Unsupported installer version: $version" }
    if (-not $SkipBuild) {
        cargo build --workspace --release --locked --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $binaryDirectory = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc\release'
    foreach ($name in @('glint.exe', 'glint-settings.exe')) {
        $binaryPath = Join-Path $binaryDirectory $name
        if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
            throw "Missing $binaryPath. Run this script without -SkipBuild first."
        }
    }
    & $IsccPath "/DAppVersion=$version" "/DProjectRoot=$projectRoot" "/DBinaryDirectory=$binaryDirectory" "/DRuntimeDirectory=$RuntimeDirectory" (Join-Path $PSScriptRoot 'installer\glint.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $installerPath = Join-Path $projectRoot "dist\Glint-v$version-windows-x64-setup.exe"
    $hash = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($installerPath))" | Set-Content -LiteralPath "$installerPath.sha256" -Encoding ascii
    Write-Output "Ready: $installerPath"
    Write-Output "SHA256: $hash"
}
finally { Pop-Location }
