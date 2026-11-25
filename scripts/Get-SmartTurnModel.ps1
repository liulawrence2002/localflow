<#
.SYNOPSIS
  Downloads the turn-completion model into a LocalFlow runtime folder (models).

.DESCRIPTION
  This is an explicit, user-run download. LocalFlow itself never downloads models and never
  contacts the network while it runs. The file comes from one pinned commit of a public
  Hugging Face repository and is verified against its SHA-256 before it replaces anything.

  Pipecat Smart Turn v3.2 (BSD-2-Clause), 32 MB. Conversation mode uses it to judge, from the
  sound of your last few seconds, whether you have finished speaking or only paused, so it can
  answer sooner after a finished sentence and wait longer after a pause mid-thought. Without
  it, conversation mode ends your turn after a fixed silence.

  This is the full-precision file, which upstream names "-gpu". It runs on the CPU here. The
  smaller 8-bit "-cpu" file is deliberately not used: LocalFlow's built-in runtime does not
  reproduce the reference answers for it.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow, which an installed LocalFlow reads.

.EXAMPLE
  .\scripts\Get-SmartTurnModel.ps1
#>
param(
  [string]$Destination,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$revision = "f766f81d3cfdf7737ac64aad813d91bbfd56bf93"
$name = "smart-turn-v3.2-gpu.onnx"
$sha256 = "ab8dc64b88713f90b571c15b714bd1330e6c883cad8763dacf65c9376dc539be"
$url = "https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/$revision/$name"

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
