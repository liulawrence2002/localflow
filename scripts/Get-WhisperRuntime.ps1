<#
.SYNOPSIS
  Downloads the whisper.cpp programs LocalFlow uses for its Whisper fallback.

.DESCRIPTION
  This is an explicit, user-run download for building LocalFlow from source; the installer
  already contains these files. LocalFlow itself never downloads anything.

  It downloads the pinned whisper.cpp Windows x64 release archive from
  github.com/ggml-org/whisper.cpp, checks its SHA-256, and extracts only the files LocalFlow
  runs: whisper-cli.exe, whisper-server.exe, whisper.dll and the ggml*.dll files. Each
  extracted file is checked against its own SHA-256 before it replaces anything. The other
  programs in the archive (SDL2.dll, parakeet.dll, the examples and tests) are left out.

  The files go to <Destination>\whisper\Release, the layout the build bundles. Other files
  already in that folder are left alone.

  The programs need the Microsoft Visual C++ runtime (msvcp140.dll, vcruntime140.dll,
  vcruntime140_1.dll, vcomp140.dll). A computer with Visual Studio or the Build Tools has it;
  the release installer ships those DLLs next to the programs.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow.

.EXAMPLE
  .\scripts\Get-WhisperRuntime.ps1
  .\scripts\Get-WhisperRuntime.ps1 -Force    # extract again even if the files match
#>
param(
  [string]$Destination,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

# whisper.cpp v1.9.5. Its Windows binaries are attached to the matching nightly build, b5454
# (same commit, d1be6fde11ac6e0407606b4e42fe72d34add8037).
$version = "v1.9.5 (build b5454)"
$url = "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip"
$archiveSha256 = "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb"
$archiveBytes = 8928640
# Archive entry "Release/<name>" -> SHA-256 of the extracted file.
$files = [ordered]@{
  "whisper-cli.exe"             = "331dba46d6427105d2b802cdbc7eae916ea1c5abf9b0d3b5cfe460d8db8e4366"
  "whisper-server.exe"          = "a97456e747f28e7bb09c1dac35f40d94404c5ecffa74aaaaa7a3753ea6543121"
  "whisper.dll"                 = "034396d75bb5720c1851674ff64a96192b58f6927b68d334ebd254ce6cad9674"
  "ggml.dll"                    = "31fe18616d872af9b6134b47a6dff8072ae4ecbe5df0273f4f8dc36766c9db8e"
  "ggml-base.dll"               = "bede0d6b2d387236ecd2b5dd676d51c8984b5b6e0ae461a12174e9c01345c5f0"
  "ggml-cpu-alderlake.dll"      = "d8f5c56ea490394eeae8b271650f2647aecd039a542d6df9e0140726e62a4603"
  "ggml-cpu-cannonlake.dll"     = "74127b40abd4f595d9db6af0fa313a02e18ff42fc228bfa05281e7c788dd4507"
  "ggml-cpu-cascadelake.dll"    = "4b11c427b844eeac87529af9a8d94b35aad2024666beb2a35e59fe3077e4b81e"
  "ggml-cpu-haswell.dll"        = "1306b1014c8118d27234d8ddc0e66ced1f8c8476bf85071595fd182ea9b7263b"
  "ggml-cpu-icelake.dll"        = "2b4dc4c66e7d8fe2807283816e9cf3b7f7d0556f59df34069871e37eab8aed40"
  "ggml-cpu-sandybridge.dll"    = "04edbdebc01a1d421062a960881a4f34d520d99693999518243f332406444c6e"
  "ggml-cpu-skylakex.dll"       = "1774714cf93621406ff35344b54715619a83242ae3e331e44d98a3b35ce801be"
  "ggml-cpu-sse42.dll"          = "f060318772c4c2dbac312bbfff6ea700b85dbc502bb38585a7186d06d549301a"
  "ggml-cpu-x64.dll"            = "4b4b05caba40d4309b5d710ef74c59ba190516b8489f27bdb1ba11ae33c6e9d3"
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
$targetDir = Join-Path $Destination "whisper\Release"
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

function Get-Sha256 {
  param([string]$Path)
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$present = $true
foreach ($name in $files.Keys) {
  $path = Join-Path $targetDir $name
  if (-not (Test-Path -LiteralPath $path) -or (Get-Sha256 $path) -ne $files[$name]) {
    $present = $false
    break
  }
}
if ($present -and -not $Force) {
  Write-Host "whisper.cpp $version is already in $targetDir"
  exit 0
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("localflow-whisper-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $work | Out-Null
try {
  $archive = Join-Path $work "whisper-bin-x64.zip"
  Write-Host "Downloading whisper.cpp $version from github.com ..."
  # curl.exe ships with Windows 10 and later; no telemetry, no user data. HTTPS only, also
  # for the redirect to GitHub's download host.
  & curl.exe --fail --location --silent --show-error --proto "=https" --proto-redir "=https" --output $archive $url
  if ($LASTEXITCODE -ne 0) { throw "Download failed (curl exit code $LASTEXITCODE)." }

  $size = (Get-Item -LiteralPath $archive).Length
  $actual = Get-Sha256 $archive
  if ($size -ne $archiveBytes -or $actual -ne $archiveSha256) {
    throw "The archive has SHA-256 $actual ($size bytes), expected $archiveSha256 ($archiveBytes bytes). Nothing was replaced."
  }

  # Extract only the named entries, each to a fixed name, so nothing in the archive can
  # choose where a file is written.
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $staging = Join-Path $work "Release"
  New-Item -ItemType Directory -Path $staging | Out-Null
  $zip = [IO.Compression.ZipFile]::OpenRead($archive)
  try {
    foreach ($name in $files.Keys) {
      $entry = $zip.GetEntry("Release/$name")
      if (-not $entry) { throw "The archive has no Release/$name. Nothing was replaced." }
      [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $staging $name), $true)
    }
  }
  finally {
    $zip.Dispose()
  }

  foreach ($name in $files.Keys) {
    $hash = Get-Sha256 (Join-Path $staging $name)
    if ($hash -ne $files[$name]) {
      throw "$name has SHA-256 $hash, expected $($files[$name]). Nothing was replaced."
    }
  }

  foreach ($name in $files.Keys) {
    Move-Item -LiteralPath (Join-Path $staging $name) -Destination (Join-Path $targetDir $name) -Force
  }
}
finally {
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Installed whisper.cpp $version ($($files.Count) files) in $targetDir"
