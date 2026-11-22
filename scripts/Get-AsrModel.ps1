<#
.SYNOPSIS
  Downloads a local speech-recognition model into .localflow-runtime\models.

.DESCRIPTION
  This is an explicit, user-run download. LocalFlow itself never downloads models and never
  contacts the network during dictation. Files come from public Hugging Face repositories
  and are verified against their expected byte size before replacing any existing file.

  parakeet  NVIDIA Parakeet TDT 0.6B v3 (CC-BY-4.0), Q5_K_M GGUF, 549 MB. The default: far
            more accurate than tiny.en and kept loaded in-process.
  tiny.en   Whisper tiny.en, 32 MB. The small fallback, also bundled with the installer.

.EXAMPLE
  .\scripts\Get-AsrModel.ps1                   # Parakeet, the default
  .\scripts\Get-AsrModel.ps1 -Model tiny.en    # the small fallback model
#>
param(
  [ValidateSet("parakeet", "tiny.en")]
  [string]$Model = "parakeet",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$catalog = @{
  "parakeet" = @{
    File  = "parakeet-tdt-0.6b-v3-Q5_K_M.gguf"
    Url   = "https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf/resolve/main/parakeet-tdt-0.6b-v3-Q5_K_M.gguf"
    Bytes = 548946272
  }
  "tiny.en"  = @{
    File  = "ggml-tiny.en-q5_1.bin"
    Url   = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en-q5_1.bin"
    Bytes = 32166155
  }
}

$entry = $catalog[$Model]
$repoRoot = Split-Path -Parent $PSScriptRoot
$modelsDir = Join-Path $repoRoot ".localflow-runtime\models"
$target = Join-Path $modelsDir $entry.File
$partial = "$target.part"

New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null

if ((Test-Path -LiteralPath $target) -and -not $Force) {
  $existing = (Get-Item -LiteralPath $target).Length
  if ($existing -eq $entry.Bytes) {
    Write-Host "Already present: $target"
    exit 0
  }
  Write-Host "Existing file has an unexpected size ($existing bytes); downloading again."
}

$megabytes = [math]::Round($entry.Bytes / 1MB)
Write-Host "Downloading $($entry.File) ($megabytes MB) from huggingface.co ..."

# curl.exe ships with Windows 10 and later; no telemetry, no user data.
& curl.exe --fail --location --silent --show-error --output $partial $entry.Url
if ($LASTEXITCODE -ne 0) {
  Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
  throw "Download failed (curl exit code $LASTEXITCODE)."
}

$downloaded = (Get-Item -LiteralPath $partial).Length
if ($downloaded -ne $entry.Bytes) {
  Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
  throw "Downloaded size $downloaded does not match the expected $($entry.Bytes) bytes."
}

Move-Item -LiteralPath $partial -Destination $target -Force
Write-Host "Saved $target"

if ($Model -eq "parakeet") {
  Write-Host "LocalFlow uses this model by default on the next dictation."
}
else {
  Write-Host "LocalFlow uses this model when Parakeet is not installed, and as the fallback."
}
