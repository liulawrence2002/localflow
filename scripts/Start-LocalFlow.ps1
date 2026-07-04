param(
  [switch]$Restart,
  [switch]$SkipOllama
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path -LiteralPath $cargoBin) -and (($env:Path -split ";") -notcontains $cargoBin)) {
  $env:Path = "$cargoBin;$env:Path"
}

function Get-ProcessPath {
  param([System.Diagnostics.Process]$Process)
  try {
    return $Process.Path
  } catch {
    return ""
  }
}

function Stop-ProcessTree {
  param([int]$ProcessId)

  $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $ProcessId" -ErrorAction SilentlyContinue
  foreach ($child in $children) {
    Stop-ProcessTree -ProcessId $child.ProcessId
  }

  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Stop-KnownViteServer {
  $pidFile = Join-Path $repoRoot ".localflow-vite.pid"
  if (Test-Path -LiteralPath $pidFile) {
    $vitePid = Get-Content -LiteralPath $pidFile -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($vitePid) {
      $process = Get-Process -Id $vitePid -ErrorAction SilentlyContinue
      if ($process) {
        Stop-ProcessTree -ProcessId $process.Id
      }
    }
    Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
  }

  $listeners = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue
  foreach ($listener in $listeners) {
    $owner = Get-CimInstance Win32_Process -Filter "ProcessId = $($listener.OwningProcess)" -ErrorAction SilentlyContinue
    $commandLine = if ($owner) { [string]$owner.CommandLine } else { "" }
    if ($commandLine.Contains($repoRoot) -or $commandLine -match "\bvite\b") {
      Stop-ProcessTree -ProcessId $listener.OwningProcess
    } else {
      Write-Warning "Port 1420 is still listening, but LocalFlow cannot identify it as this repo's Vite process. Leaving it alone."
    }
  }
}

function Ensure-ReleaseBuild {
  param([string]$ExePath)

  if (Test-Path -LiteralPath $ExePath) {
    return
  }

  if (-not (Test-Path -LiteralPath "node_modules")) {
    npm install
  }

  npm run tauri:build
}

function Ensure-Ollama {
  if ($SkipOllama) {
    return
  }

  $listener = Get-NetTCPConnection -LocalPort 11434 -State Listen -ErrorAction SilentlyContinue
  if ($listener) {
    return
  }

  $ollama = Get-Command ollama.exe -ErrorAction SilentlyContinue
  if (-not $ollama) {
    Write-Warning "Ollama is not on PATH. LocalFlow will still run, but model cleanup may fall back to deterministic text."
    return
  }

  Start-Process -FilePath $ollama.Source -ArgumentList @("serve") -WindowStyle Hidden
  Start-Sleep -Seconds 2
}

Stop-KnownViteServer
Ensure-Ollama

$releaseExe = Join-Path $repoRoot "src-tauri\target\release\localflow.exe"
Ensure-ReleaseBuild -ExePath $releaseExe
$resolvedReleaseExe = (Resolve-Path -LiteralPath $releaseExe).Path

Get-Process -Name localflow -ErrorAction SilentlyContinue |
  Where-Object { (Get-ProcessPath $_) -like "*\target\debug\localflow.exe" } |
  Stop-Process -Force

$existingRelease = @(
  Get-Process -Name localflow -ErrorAction SilentlyContinue |
    Where-Object { (Get-ProcessPath $_) -eq $resolvedReleaseExe }
)
$existing = $existingRelease | Select-Object -First 1

if ($existing -and -not $Restart) {
  $existingRelease | Select-Object -Skip 1 | Stop-Process -Force
  Set-Content -LiteralPath ".localflow-release.pid" -Value $existing.Id
  Write-Host "LocalFlow release app is already running as PID $($existing.Id)."
  Write-Host "No Vite dev server was started."
  return
}

if ($existingRelease.Count -gt 0 -and $Restart) {
  $existingRelease | Stop-Process -Force
  Start-Sleep -Milliseconds 500
}

$workingDirectory = Split-Path -Parent $resolvedReleaseExe
$process = Start-Process -FilePath $resolvedReleaseExe -WorkingDirectory $workingDirectory -WindowStyle Hidden -PassThru
Set-Content -LiteralPath ".localflow-release.pid" -Value $process.Id

Write-Host "LocalFlow release app started as PID $($process.Id)."
Write-Host "No Vite dev server was started."
