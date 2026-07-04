param(
  [switch]$Install
)

$ErrorActionPreference = "Stop"

function Test-Command($Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Add-CargoPath {
  $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
  if ((Test-Path -LiteralPath $cargoBin) -and (($env:Path -split ";") -notcontains $cargoBin)) {
    $env:Path = "$cargoBin;$env:Path"
  }
}

function Test-MSVCBuildTools {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path -LiteralPath $vswhere)) {
    return $false
  }

  $installPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  return [bool]$installPath
}

Add-CargoPath

$checks = @(
  @{ Name = "node"; InstallId = "OpenJS.NodeJS.LTS" },
  @{ Name = "npm"; InstallId = "OpenJS.NodeJS.LTS" },
  @{ Name = "git"; InstallId = "Git.Git" },
  @{ Name = "cargo"; InstallId = "Rustlang.Rustup" },
  @{ Name = "rustc"; InstallId = "Rustlang.Rustup" }
)

foreach ($check in $checks) {
  if (Test-Command $check.Name) {
    Write-Host "$($check.Name): found"
    continue
  }

  Write-Host "$($check.Name): missing"
  if ($Install) {
    if (-not (Test-Command "winget")) {
      throw "winget is not available. Install $($check.InstallId) manually."
    }
    winget install --id $check.InstallId --exact --silent --accept-package-agreements --accept-source-agreements
    Add-CargoPath
  }
}

if (Test-MSVCBuildTools) {
  Write-Host "MSVC Build Tools: found"
} else {
  Write-Host "MSVC Build Tools: missing"
  if ($Install) {
    if (-not (Test-Command "winget")) {
      throw "winget is not available. Install Microsoft.VisualStudio.2022.BuildTools manually."
    }
    winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --silent --accept-package-agreements --accept-source-agreements --override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --add Microsoft.VisualStudio.Component.Windows11SDK.26100"
  }
}

Write-Host "WebView2 Runtime is also required for Tauri on Windows 11."
