param(
    [string]$Version = "0.1.0",
    [string]$OutputRoot = "release"
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseExe = Join-Path $projectRoot "src-tauri\target\release\ui-studio.exe"
$outputBase = Join-Path $projectRoot $OutputRoot
$packageName = "UI-Studio-$Version-windows-x64"
$stage = Join-Path $outputBase $packageName
$archive = Join-Path $outputBase "$packageName.zip"
$resolvedProject = [IO.Path]::GetFullPath($projectRoot).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$resolvedOutput = [IO.Path]::GetFullPath($outputBase).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$resolvedStage = [IO.Path]::GetFullPath($stage)

if (-not $resolvedOutput.StartsWith($resolvedProject, [StringComparison]::OrdinalIgnoreCase) -or
    -not $resolvedStage.StartsWith($resolvedOutput, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Portable output must stay inside the project release directory."
}

if (-not (Test-Path -LiteralPath $releaseExe -PathType Leaf)) {
    throw "Release executable not found. Run 'npm run tauri -- build --no-bundle' first."
}

if (Test-Path -LiteralPath $stage) {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}

New-Item -ItemType Directory -Path $stage | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage "tools") | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage "resources") | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage "runtime") | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage "models") | Out-Null

Copy-Item -LiteralPath $releaseExe -Destination (Join-Path $stage "UI-Studio.exe")
Copy-Item -LiteralPath (Join-Path $projectRoot "tools\ui_studio_worker.py") -Destination (Join-Path $stage "tools\ui_studio_worker.py")
Copy-Item -LiteralPath (Join-Path $projectRoot "resources\resource-manifest.json") -Destination (Join-Path $stage "resources\resource-manifest.json")
Copy-Item -LiteralPath (Join-Path $projectRoot "README.md") -Destination (Join-Path $stage "README.md")
Copy-Item -LiteralPath (Join-Path $projectRoot "PORTABLE.md") -Destination (Join-Path $stage "PORTABLE.md")

Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal
$hash = Get-FileHash -LiteralPath $archive -Algorithm SHA256
$hashLine = "$($hash.Hash.ToLowerInvariant())  $packageName.zip"
Set-Content -LiteralPath "$archive.sha256" -Value $hashLine -Encoding ascii

Get-Item -LiteralPath $archive, "$archive.sha256" | Select-Object FullName, Length, LastWriteTime
