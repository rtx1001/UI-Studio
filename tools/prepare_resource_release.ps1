param(
    [string]$OutputRoot = "release\resource-assets"
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$dreamyRoot = Split-Path -Parent $projectRoot
$runtimeSource = Join-Path $dreamyRoot "external\DiffSynth-Studio"
$pythonSource = "C:\Program Files\Python310"
$loraSource = Join-Path $dreamyRoot "training\runs\dreamroom-object-detail-full-v3-flux2-klein-4b-r40-768\DreamRoom-Objects-v1.safetensors"
$outputBase = Join-Path $projectRoot $OutputRoot
$resolvedProject = [IO.Path]::GetFullPath($projectRoot).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$resolvedOutput = [IO.Path]::GetFullPath($outputBase)

if (-not $resolvedOutput.StartsWith($resolvedProject, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Resource output must remain inside the UI Studio project."
}
if (-not (Test-Path -LiteralPath (Join-Path $runtimeSource ".venv\Scripts\python.exe") -PathType Leaf)) {
    throw "The validated FLUX Python environment is missing."
}
if (-not (Test-Path -LiteralPath (Join-Path $pythonSource "python.exe") -PathType Leaf)) {
    throw "The Python 3.10 base runtime is missing."
}
if (-not (Test-Path -LiteralPath (Join-Path $runtimeSource "diffsynth") -PathType Container)) {
    throw "The DiffSynth package is missing."
}
if (-not (Test-Path -LiteralPath $loraSource -PathType Leaf)) {
    throw "The DreamRoom LoRA is missing."
}

New-Item -ItemType Directory -Path $outputBase -Force | Out-Null
$runtimeStage = Join-Path $outputBase "runtime-stage"
$runtimeArchive = Join-Path $outputBase "UI-Studio-FLUX-Runtime-windows-x64.zip"
$loraAsset = Join-Path $outputBase "DreamRoom-Objects-v1.safetensors"

foreach ($target in @($runtimeStage, $runtimeArchive, $loraAsset)) {
    $resolvedTarget = [IO.Path]::GetFullPath($target)
    if (-not $resolvedTarget.StartsWith($resolvedOutput, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to replace a path outside the resource output directory: $resolvedTarget"
    }
    if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Recurse -Force
    }
}

$runtimeDestination = Join-Path $runtimeStage "runtime\diffsynth"
$portablePython = Join-Path $runtimeDestination "python"
New-Item -ItemType Directory -Path $portablePython -Force | Out-Null

# A Windows venv is not portable: its launcher still resolves the Python base
# installation recorded in pyvenv.cfg. Build a standalone runtime from the
# Python base files and overlay the validated environment's site-packages.
foreach ($name in @("DLLs", "tcl")) {
    $source = Join-Path $pythonSource $name
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination $portablePython -Recurse
    }
}
New-Item -ItemType Directory -Path (Join-Path $portablePython "Lib") -Force | Out-Null
Get-ChildItem -LiteralPath (Join-Path $pythonSource "Lib") -Force |
    Where-Object { $_.Name -ne "site-packages" } |
    Copy-Item -Destination (Join-Path $portablePython "Lib") -Recurse
Copy-Item -LiteralPath (Join-Path $runtimeSource ".venv\Lib\site-packages") -Destination (Join-Path $portablePython "Lib") -Recurse
foreach ($name in @("LICENSE.txt", "python.exe", "pythonw.exe", "python3.dll", "python310.dll", "vcruntime140.dll", "vcruntime140_1.dll")) {
    $source = Join-Path $pythonSource $name
    if (Test-Path -LiteralPath $source -PathType Leaf) {
        Copy-Item -LiteralPath $source -Destination $portablePython
    }
}
Copy-Item -LiteralPath (Join-Path $runtimeSource "diffsynth") -Destination $runtimeDestination -Recurse
if (Test-Path -LiteralPath (Join-Path $runtimeSource "diffsynth.egg-info")) {
    Copy-Item -LiteralPath (Join-Path $runtimeSource "diffsynth.egg-info") -Destination $runtimeDestination -Recurse
}
foreach ($name in @("LICENSE", "pyproject.toml")) {
    $source = Join-Path $runtimeSource $name
    if (Test-Path -LiteralPath $source -PathType Leaf) {
        Copy-Item -LiteralPath $source -Destination $runtimeDestination
    }
}

Copy-Item -LiteralPath $loraSource -Destination $loraAsset

& tar.exe -a -c -f $runtimeArchive -C $runtimeStage .
if ($LASTEXITCODE -ne 0) { throw "Failed to create the FLUX runtime archive." }

$partSize = 1900MB
$partAssets = @()
$input = [IO.File]::OpenRead($runtimeArchive)
try {
    $partNumber = 1
    $buffer = New-Object byte[] (8MB)
    while ($input.Position -lt $input.Length) {
        $partPath = "$runtimeArchive.part$($partNumber.ToString('00'))"
        $output = [IO.File]::Create($partPath)
        try {
            $remaining = [Math]::Min([int64]$partSize, $input.Length - $input.Position)
            while ($remaining -gt 0) {
                $read = $input.Read($buffer, 0, [int][Math]::Min($buffer.Length, $remaining))
                if ($read -le 0) { throw "Unexpected end of runtime archive while splitting." }
                $output.Write($buffer, 0, $read)
                $remaining -= $read
            }
        } finally {
            $output.Dispose()
        }
        $partAssets += $partPath
        $partNumber++
    }
} finally {
    $input.Dispose()
}

$assets = @($partAssets) + @($loraAsset) | ForEach-Object {
    $file = Get-Item -LiteralPath $_
    [pscustomobject]@{
        name = $file.Name
        bytes = $file.Length
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        extractTo = "."
    }
}
$metadata = [pscustomobject]@{
    runtimeArchive = [pscustomobject]@{
        name = (Split-Path -Leaf $runtimeArchive)
        bytes = (Get-Item -LiteralPath $runtimeArchive).Length
        sha256 = (Get-FileHash -LiteralPath $runtimeArchive -Algorithm SHA256).Hash.ToLowerInvariant()
        parts = @($partAssets | ForEach-Object { Split-Path -Leaf $_ })
        extractTo = "."
    }
    assets = $assets
}
$metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputBase "release-assets.json") -Encoding utf8

Remove-Item -LiteralPath $runtimeStage -Recurse -Force
Remove-Item -LiteralPath $runtimeArchive -Force
@($partAssets) + @($loraAsset, (Join-Path $outputBase "release-assets.json")) |
    ForEach-Object { Get-Item -LiteralPath $_ } |
    Select-Object FullName, Length, LastWriteTime
