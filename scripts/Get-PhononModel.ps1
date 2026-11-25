<#
.SYNOPSIS
  Downloads the fast conversation voice into a LocalFlow runtime folder (models\phonon).

.DESCRIPTION
  This is an explicit, user-run download. LocalFlow itself never downloads models and never
  contacts the network while it runs. Files come from one pinned commit of a public Hugging
  Face repository, and each is verified against its SHA-256 before it replaces anything.

  Phonon 31m (English) by Gradium: Apache-2.0 weights and default voice, about 65 MB in
  total. It runs on the CPU, faster than real time, and streams audio as it is generated.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow, which an installed LocalFlow reads.

.EXAMPLE
  .\scripts\Get-PhononModel.ps1
  .\scripts\Get-PhononModel.ps1 -Force    # download again even if the files are present
#>
param(
  [string]$Destination,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$revision = "92741c5b07d085ff840e26fea8ce43110b87852f"
$baseUrl = "https://huggingface.co/gradium/phonon-31m-en/resolve/$revision"
$files = @(
  @{ Name = "config.json"; Sha256 = "b5d0f88353638370d94035b7d73d7d6577c0554d78f8d7ecfd40490aae39ae85" },
  @{ Name = "tokenizer.json"; Sha256 = "691a00068fa7b989f2f3e5ae5daa2d1a5bb319f2d5360deafcfb19a8bb6e7936" },
  @{ Name = "default-voice.safetensors"; Sha256 = "dc537478eaadc1a5257fcda2e3501ab81e14c111a5d95a5838f350db2afd0f1a" },
  @{ Name = "model.q8.gguf"; Sha256 = "a7d9aad2cd6dc66473f24a08bc6d2939030ef4d751ef82349361fbef3272dbae" }
)

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
$modelDir = Join-Path $Destination "models\phonon"
New-Item -ItemType Directory -Force -Path $modelDir | Out-Null

function Get-Sha256 {
  param([string]$Path)
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

foreach ($file in $files) {
  $target = Join-Path $modelDir $file.Name
  $partial = "$target.part"

  if ((Test-Path -LiteralPath $target) -and -not $Force) {
    if ((Get-Sha256 $target) -eq $file.Sha256) {
      Write-Host "Already present: $($file.Name)"
      continue
    }
    Write-Host "$($file.Name) does not match its expected hash; downloading again."
  }

  Write-Host "Downloading $($file.Name) from huggingface.co ..."
  # curl.exe ships with Windows 10 and later; no telemetry, no user data. HTTPS only, also
  # for the redirect to Hugging Face's file host.
  & curl.exe --fail --location --silent --show-error --proto "=https" --proto-redir "=https" --output $partial "$baseUrl/$($file.Name)"
  if ($LASTEXITCODE -ne 0) {
    Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
    throw "Download of $($file.Name) failed (curl exit code $LASTEXITCODE)."
  }

  $actual = Get-Sha256 $partial
  if ($actual -ne $file.Sha256) {
    Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
    throw "$($file.Name) has SHA-256 $actual, expected $($file.Sha256). Nothing was replaced."
  }

  Move-Item -LiteralPath $partial -Destination $target -Force
}

Write-Host "The conversation voice is installed in $modelDir"
