<#
.SYNOPSIS
  Downloads the voice-activity model into a LocalFlow runtime folder (models).

.DESCRIPTION
  This is an explicit, user-run download. LocalFlow itself never downloads models and never
  contacts the network while it runs. The file comes from one pinned commit of a public
  Hugging Face repository and is verified against its SHA-256 before it replaces anything.

  Silero VAD v6.2 (MIT), 1.2 MB. Conversation mode uses it to tell speech from background
  noise and to find the end of what you said. Without it, conversation mode falls back to a
  simple loudness rule.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow, which an installed LocalFlow reads.

.EXAMPLE
  .\scripts\Get-VadModel.ps1
#>
param(
  [string]$Destination,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$revision = "a9b2d61c20ae4ddf3644edb34969e8185a485448"
$name = "silero-vad-v6.2-F32.gguf"
$sha256 = "0d871a800c420fdd5d1b17a3aea4296d1c39608f0ff4a9295eb76457eb1c88e6"
$url = "https://huggingface.co/handy-computer/silero-vad-v6.2-gguf/resolve/$revision/$name"

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
$target = Join-Path $modelsDir $name
$partial = "$target.part"
New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null

function Get-Sha256 {
  param([string]$Path)
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

if ((Test-Path -LiteralPath $target) -and -not $Force -and ((Get-Sha256 $target) -eq $sha256)) {
  Write-Host "Already present: $target"
  exit 0
}

Write-Host "Downloading $name from huggingface.co ..."
# curl.exe ships with Windows 10 and later; no telemetry, no user data. HTTPS only, also
# for the redirect to Hugging Face's file host.
& curl.exe --fail --location --silent --show-error --proto "=https" --proto-redir "=https" --output $partial $url
if ($LASTEXITCODE -ne 0) {
  Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
  throw "Download failed (curl exit code $LASTEXITCODE)."
}

$actual = Get-Sha256 $partial
if ($actual -ne $sha256) {
  Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
  throw "$name has SHA-256 $actual, expected $sha256. Nothing was replaced."
}

Move-Item -LiteralPath $partial -Destination $target -Force
Write-Host "Saved $target"
