param(
  [switch]$Install
)

$ErrorActionPreference = "Stop"

function Test-Command($Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

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
  }
}

Write-Host "Tauri Windows prerequisites may also require Microsoft C++ Build Tools and WebView2 Runtime."
