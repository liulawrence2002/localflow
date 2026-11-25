<#
.SYNOPSIS
  Downloads a local speech-recognition model.

.DESCRIPTION
  This is an explicit, user-run download. LocalFlow itself never downloads models and never
  contacts the network during dictation. Each file comes from one pinned commit of a public
  Hugging Face repository and is checked against its SHA-256 before it replaces anything.

  parakeet  NVIDIA Parakeet TDT 0.6B v3 (CC BY 4.0), Q5_K_M GGUF, 549 MB. The default: far
            more accurate than tiny.en and kept loaded in-process.
  tiny.en   Whisper tiny.en (MIT), 32 MB. The small fallback. The installer already contains
            it; building from source needs it.

  The file goes to <Destination>\models.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow, which an installed LocalFlow reads.

.EXAMPLE
  .\scripts\Get-AsrModel.ps1                   # Parakeet, the default
  .\scripts\Get-AsrModel.ps1 -Model tiny.en    # the small fallback model
#>
param(
  [ValidateSet("parakeet", "tiny.en")]
  [string]$Model = "parakeet",
  [string]$Destination,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$catalog = @{
  "parakeet" = @{
    File   = "parakeet-tdt-0.6b-v3-Q5_K_M.gguf"
    Repo   = "handy-computer/parakeet-tdt-0.6b-v3-gguf"
    Commit = "d8d8c32cb2017c57e5307767343e97870f8a7f53"
    Sha256 = "cc722e76adc1a629fc0b2535de879d99b8160d07ad4c0215e2ca7d7ea0ae4b8f"
    Bytes  = 548946272
  }
  "tiny.en"  = @{
    File   = "ggml-tiny.en-q5_1.bin"
    Repo   = "ggerganov/whisper.cpp"
    Commit = "5359861c739e955e79d9a303bcbc70fb988958b1"
    Sha256 = "c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b"
    Bytes  = 32166155
  }
}
$entry = $catalog[$Model]
$url = "https://huggingface.co/$($entry.Repo)/resolve/$($entry.Commit)/$($entry.File)"

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
$modelsDir = Join-Path $Destination "models"
$target = Join-Path $modelsDir $entry.File
$partial = "$target.part"
New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null

function Get-Sha256 {
  param([string]$Path)
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

if ((Test-Path -LiteralPath $target) -and -not $Force) {
  if ((Get-Item -LiteralPath $target).Length -eq $entry.Bytes -and (Get-Sha256 $target) -eq $entry.Sha256) {
    Write-Host "Already present: $target"
    exit 0
  }
  Write-Host "$($entry.File) does not match its expected SHA-256; downloading again."
}

$megabytes = [math]::Round($entry.Bytes / 1MB)
Write-Host "Downloading $($entry.File) ($megabytes MB) from huggingface.co ..."

# curl.exe ships with Windows 10 and later; no telemetry, no user data. HTTPS only, also for
# the redirect to Hugging Face's file host.
& curl.exe --fail --location --progress-bar --show-error --proto "=https" --proto-redir "=https" --output $partial $url
if ($LASTEXITCODE -ne 0) {
  Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
  throw "Download failed (curl exit code $LASTEXITCODE)."
}

$actual = Get-Sha256 $partial
if ($actual -ne $entry.Sha256) {
  Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
  throw "$($entry.File) has SHA-256 $actual, expected $($entry.Sha256). Nothing was replaced."
}

Move-Item -LiteralPath $partial -Destination $target -Force
Write-Host "Saved $target"

if ($Model -eq "parakeet") {
  Write-Host "LocalFlow uses this model by default on the next dictation."
}
else {
  Write-Host "LocalFlow uses this model when Parakeet is not installed, and as the fallback."
}
