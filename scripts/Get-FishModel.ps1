<#
.SYNOPSIS
  Downloads the Fish Audio S2 Pro voice model (GGUF) into .localflow-runtime\models\fish.

.DESCRIPTION
  This is an explicit, user-run download; LocalFlow never downloads models by itself. Files
  come from the community GGUF conversion at https://huggingface.co/rodrigomt/s2-pro-gguf and
  are verified against their expected size.

  Only one quantisation is kept: any other s2-pro-*.gguf in the folder is removed after a
  successful download so the multi-gigabyte files do not pile up.

  The S2 Pro weights are under the Fish Audio Research License: research and personal
  non-commercial use only. Commercial use needs a separate licence from Fish Audio.

.EXAMPLE
  .\scripts\Get-FishModel.ps1                 # q4_k_m, 3.6 GB: the best compact variant
  .\scripts\Get-FishModel.ps1 -Quant q6_k     # 4.5 GB: better quality, needs 6+ GB of VRAM
#>
param(
  [ValidateSet("q2_k", "q3_k", "q4_k_m", "q5_k_m", "q6_k", "q8_0")]
  [string]$Quant = "q4_k_m",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$base = "https://huggingface.co/rodrigomt/s2-pro-gguf/resolve/main"
$sizes = @{
  "q2_k"   = 2576474208
  "q3_k"   = 3029197408
  "q4_k_m" = 3566165088
  "q5_k_m" = 4031183968
  "q6_k"   = 4525266528
  "q8_0"   = 5630037088
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$modelDir = Join-Path $repoRoot ".localflow-runtime\models\fish"
New-Item -ItemType Directory -Force -Path $modelDir | Out-Null

function Get-RemoteFile([string]$name, [long]$expectedBytes) {
  $target = Join-Path $modelDir $name
  if ((Test-Path -LiteralPath $target) -and -not $Force) {
    if ($expectedBytes -le 0 -or (Get-Item -LiteralPath $target).Length -eq $expectedBytes) {
      Write-Host "Already present: $name"
      return
    }
  }

  $partial = "$target.part"
  Write-Host "Downloading $name ..."
  & curl.exe --fail --location --silent --show-error --output $partial "$base/$name"
  if ($LASTEXITCODE -ne 0) {
    Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
    throw "Download of $name failed (curl exit code $LASTEXITCODE)."
  }
  $downloaded = (Get-Item -LiteralPath $partial).Length
  if ($expectedBytes -gt 0 -and $downloaded -ne $expectedBytes) {
    Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
    throw "$name is $downloaded bytes; expected $expectedBytes."
  }
  Move-Item -LiteralPath $partial -Destination $target -Force
}

$modelName = "s2-pro-$Quant.gguf"
$free = (Get-PSDrive -Name (Split-Path -Qualifier $modelDir).TrimEnd(":")).Free
$needed = $sizes[$Quant]
if (-not (Test-Path -LiteralPath (Join-Path $modelDir $modelName)) -and $free -lt ($needed + 1GB)) {
  throw "Not enough free disk space: $([math]::Round($needed / 1GB, 1)) GB needed plus 1 GB of headroom, $([math]::Round($free / 1GB, 1)) GB free."
}

Get-RemoteFile "tokenizer.json" 0
Get-RemoteFile "LICENSE.md" 0
Get-RemoteFile $modelName $needed

# Keep exactly one quantisation.
Get-ChildItem -LiteralPath $modelDir -Filter "s2-pro-*.gguf" |
  Where-Object { $_.Name -ne $modelName } |
  ForEach-Object {
    Write-Host "Removing other quantisation: $($_.Name)"
    Remove-Item -LiteralPath $_.FullName -Force
  }

Write-Host ""
Write-Host "Fish S2 Pro ($Quant) is in $modelDir"
Write-Host "Licence: Fish Audio Research License (research and personal non-commercial use only)."
Write-Host "Build the engine with .\scripts\Build-FishS2.ps1 if you have not yet."
