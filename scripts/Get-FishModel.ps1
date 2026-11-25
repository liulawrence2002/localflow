<#
.SYNOPSIS
  Downloads the Fish Audio S2 Pro voice model (GGUF), the optional read-aloud voice.

.DESCRIPTION
  This is an explicit, user-run download; LocalFlow never downloads models by itself. Files
  come from one pinned commit of the community GGUF conversion at
  https://huggingface.co/rodrigomt/s2-pro-gguf, and each is checked against its SHA-256
  before it replaces anything.

  The files go to <Destination>\models\fish. Only one quantisation is kept: any other
  s2-pro-*.gguf in that folder is removed after a successful download so the multi-gigabyte
  files do not pile up.

  The S2 Pro weights are under the Fish Audio Research License: research and personal
  non-commercial use only. Commercial use needs a separate licence from Fish Audio. The
  licence is saved next to the model as LICENSE.md.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow, which an installed LocalFlow reads. Build the
  engine (Build-FishS2.ps1) into the same folder.

.EXAMPLE
  .\scripts\Get-FishModel.ps1                 # q4_k_m, 3.6 GB: the best compact variant
  .\scripts\Get-FishModel.ps1 -Quant q6_k     # 4.5 GB: better quality, needs 6+ GB of VRAM
#>
param(
  [ValidateSet("q2_k", "q3_k", "q4_k_m", "q5_k_m", "q6_k", "q8_0")]
  [string]$Quant = "q4_k_m",
  [string]$Destination,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$commit = "a7320690b5585b03b20ed6484b55926f3015f48d"
$base = "https://huggingface.co/rodrigomt/s2-pro-gguf/resolve/$commit"
$catalog = @{
  "tokenizer.json"     = @{ Bytes = 12217872; Sha256 = "f24e08099d45a8adf3f52f5f0b03276e433bb9d689bb15fcbcc48ce58744588b" }
  "LICENSE.md"         = @{ Bytes = 10360; Sha256 = "aa7d9206e9d710590987a3636934f643529c00cd490323594e6206aaa0c32d80" }
  "s2-pro-q2_k.gguf"   = @{ Bytes = 2576474208; Sha256 = "38f888f1358725fa026884c69144bcf0aab1d22f76edf2a0b71f95db86d20ad9" }
  "s2-pro-q3_k.gguf"   = @{ Bytes = 3029197408; Sha256 = "9c6c976c391335105b9b462568f8e7e9eada5cb6b081b41d550cb26f74cce105" }
  "s2-pro-q4_k_m.gguf" = @{ Bytes = 3566165088; Sha256 = "83963e1b7cec980b41eb2163d617e2b6241bfd1564dd880e5b43fc4834807bd9" }
  "s2-pro-q5_k_m.gguf" = @{ Bytes = 4031183968; Sha256 = "e445b0c8f32ed0ff584b906098f0fe53a67c0691249bfcccde569544f7d72cb9" }
  "s2-pro-q6_k.gguf"   = @{ Bytes = 4525266528; Sha256 = "84ac904172a2cadb84e8f7f14ea3f1acef0584987635e85f7207fd254eafa235" }
  "s2-pro-q8_0.gguf"   = @{ Bytes = 5630037088; Sha256 = "e2043182234786e7b975547d3bbcb23ff02e4ff684b82f7fa851287e4cb4f267" }
}

if (-not $Destination) {
  # A checkout keeps its runtime next to the source; an installed copy reads the per-user
  # folder.
  $checkout = Split-Path -Parent $PSScriptRoot
  if (Test-Path -LiteralPath (Join-Path $checkout ".git")) {
    $Destination = Join-Path $checkout ".localflow-runtime"
  }
  else {
    $Destination = Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) "LocalFlow"
  }
}
$modelDir = Join-Path $Destination "models\fish"
New-Item -ItemType Directory -Force -Path $modelDir | Out-Null

function Get-Sha256 {
  param([string]$Path)
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-VerifiedFile([string]$name) {
  $expected = $catalog[$name]
  $target = Join-Path $modelDir $name
  if ((Test-Path -LiteralPath $target) -and -not $Force) {
    # The size check is instant; the hash of a multi-gigabyte file takes a few seconds.
    if ((Get-Item -LiteralPath $target).Length -eq $expected.Bytes -and (Get-Sha256 $target) -eq $expected.Sha256) {
      Write-Host "Already present: $name"
      return
    }
    Write-Host "$name does not match its expected SHA-256; downloading again."
  }

  $partial = "$target.part"
  Write-Host "Downloading $name ($([math]::Round($expected.Bytes / 1MB)) MB) from huggingface.co ..."
  # curl.exe ships with Windows 10 and later; no telemetry, no user data. HTTPS only, also
  # for the redirect to Hugging Face's file host.
  & curl.exe --fail --location --progress-bar --show-error --proto "=https" --proto-redir "=https" --output $partial "$base/$name"
  if ($LASTEXITCODE -ne 0) {
    Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
    throw "Download of $name failed (curl exit code $LASTEXITCODE)."
  }
  $actual = Get-Sha256 $partial
  if ($actual -ne $expected.Sha256) {
    Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
    throw "$name has SHA-256 $actual, expected $($expected.Sha256). Nothing was replaced."
  }
  Move-Item -LiteralPath $partial -Destination $target -Force
}

$modelName = "s2-pro-$Quant.gguf"
$free = (Get-PSDrive -Name (Split-Path -Qualifier $modelDir).TrimEnd(":")).Free
$needed = $catalog[$modelName].Bytes
if (-not (Test-Path -LiteralPath (Join-Path $modelDir $modelName)) -and $free -lt ($needed + 1GB)) {
  throw "Not enough free disk space: $([math]::Round($needed / 1GB, 1)) GB needed plus 1 GB of headroom, $([math]::Round($free / 1GB, 1)) GB free."
}

Get-VerifiedFile "tokenizer.json"
Get-VerifiedFile "LICENSE.md"
Get-VerifiedFile $modelName

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
Write-Host "Build the engine into the same folder with .\scripts\Build-FishS2.ps1 if you have not yet."
