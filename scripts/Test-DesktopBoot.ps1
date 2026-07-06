param(
  [string]$ShortcutPath = (Join-Path ([Environment]::GetFolderPath("Desktop")) "LocalFlow.lnk"),
  [switch]$RequireRunning,
  [int]$ExpectRecentHotkeySeconds = 0
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$releaseExe = Join-Path $repoRoot "src-tauri\target\release\localflow.exe"
$vbsLauncher = Join-Path $repoRoot "scripts\Start-LocalFlow.vbs"
$psLauncher = Join-Path $repoRoot "scripts\Start-LocalFlow.ps1"
$tauriConfigPath = Join-Path $repoRoot "src-tauri\tauri.conf.json"
$hotkeysPath = Join-Path $repoRoot "src-tauri\src\hotkeys\mod.rs"
$libPath = Join-Path $repoRoot "src-tauri\src\lib.rs"
$mainTsxPath = Join-Path $repoRoot "src\main.tsx"
$appDataDir = Join-Path $env:APPDATA "app.localflow.desktop"
$desktopHealthPath = Join-Path $appDataDir "desktop-health.json"

$checks = New-Object System.Collections.Generic.List[object]

function Add-Check {
  param(
    [string]$Name,
    [ValidateSet("PASS", "WARN", "FAIL")]
    [string]$Status,
    [string]$Details
  )

  $script:checks.Add([pscustomobject]@{
      Status  = $Status
      Check   = $Name
      Details = $Details
    })
}

function Resolve-ExistingPath {
  param([string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) {
    return $Path
  }

  return (Resolve-Path -LiteralPath $Path).Path
}

function Test-Contains {
  param(
    [string]$Text,
    [string]$Pattern
  )

  return $Text.IndexOf($Pattern, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

function Get-ProcessPath {
  param([System.Diagnostics.Process]$Process)

  try {
    return $Process.Path
  } catch {
    return ""
  }
}

function Read-DesktopHealth {
  if (-not (Test-Path -LiteralPath $desktopHealthPath)) {
    return $null
  }

  try {
    return Get-Content -Raw -LiteralPath $desktopHealthPath | ConvertFrom-Json
  } catch {
    Add-Check "Desktop health file parses" "FAIL" "Could not parse $desktopHealthPath`: $($_.Exception.Message)"
    return $null
  }
}

function Test-RecentTimestamp {
  param(
    [string]$Name,
    [object]$Timestamp,
    [int]$MaxAgeSeconds
  )

  if (-not $Timestamp) {
    Add-Check $Name "FAIL" "No timestamp recorded."
    return
  }

  try {
    $observedAt = [DateTimeOffset]::Parse([string]$Timestamp).ToUniversalTime()
    $ageSeconds = ([DateTimeOffset]::UtcNow - $observedAt).TotalSeconds
    if ($ageSeconds -le $MaxAgeSeconds) {
      Add-Check $Name "PASS" "Observed $([math]::Round($ageSeconds, 1)) seconds ago."
    } else {
      Add-Check $Name "FAIL" "Last observed $([math]::Round($ageSeconds, 1)) seconds ago; expected <= $MaxAgeSeconds."
    }
  } catch {
    Add-Check $Name "FAIL" "Invalid timestamp: $Timestamp"
  }
}

function Test-ShortcutChain {
  if (-not (Test-Path -LiteralPath $ShortcutPath)) {
    Add-Check "Desktop shortcut exists" "FAIL" "Missing shortcut: $ShortcutPath"
    return
  }

  Add-Check "Desktop shortcut exists" "PASS" $ShortcutPath

  $shell = New-Object -ComObject WScript.Shell
  $shortcut = $shell.CreateShortcut($ShortcutPath)
  $targetLeaf = Split-Path -Leaf $shortcut.TargetPath
  $expectedVbs = Resolve-ExistingPath $vbsLauncher
  $workingDirectory = Resolve-ExistingPath $shortcut.WorkingDirectory
  $expectedWorkingDirectory = Resolve-ExistingPath $repoRoot

  if ($targetLeaf -ieq "wscript.exe") {
    Add-Check "Shortcut target uses hidden WScript launcher" "PASS" $shortcut.TargetPath
  } else {
    Add-Check "Shortcut target uses hidden WScript launcher" "FAIL" "Target is $($shortcut.TargetPath)"
  }

  if (Test-Contains $shortcut.Arguments $expectedVbs) {
    Add-Check "Shortcut points at Start-LocalFlow.vbs" "PASS" $shortcut.Arguments
  } else {
    Add-Check "Shortcut points at Start-LocalFlow.vbs" "FAIL" "Arguments are $($shortcut.Arguments)"
  }

  if ($workingDirectory -ieq $expectedWorkingDirectory) {
    Add-Check "Shortcut working directory is repo root" "PASS" $shortcut.WorkingDirectory
  } else {
    Add-Check "Shortcut working directory is repo root" "FAIL" "WorkingDirectory is $($shortcut.WorkingDirectory)"
  }
}

function Test-VbsLauncher {
  if (-not (Test-Path -LiteralPath $vbsLauncher)) {
    Add-Check "VBS launcher exists" "FAIL" "Missing $vbsLauncher"
    return
  }

  $vbs = Get-Content -Raw -LiteralPath $vbsLauncher
  Add-Check "VBS launcher exists" "PASS" $vbsLauncher

  if ((Test-Contains $vbs "Start-LocalFlow.ps1") -and (Test-Contains $vbs "powershell.exe -NoProfile -ExecutionPolicy Bypass -File")) {
    Add-Check "VBS launches PowerShell launcher" "PASS" "Start-LocalFlow.ps1 via powershell.exe"
  } else {
    Add-Check "VBS launches PowerShell launcher" "FAIL" "Expected hidden PowerShell command was not found."
  }

  if (Test-Contains $vbs "shell.Run command, 0, False") {
    Add-Check "VBS hides terminal window" "PASS" "shell.Run uses window style 0"
  } else {
    Add-Check "VBS hides terminal window" "FAIL" "shell.Run command, 0, False not found."
  }
}

function Test-PowerShellLauncher {
  if (-not (Test-Path -LiteralPath $psLauncher)) {
    Add-Check "PowerShell launcher exists" "FAIL" "Missing $psLauncher"
    return
  }

  $launcher = Get-Content -Raw -LiteralPath $psLauncher
  Add-Check "PowerShell launcher exists" "PASS" $psLauncher

  if ((Test-Contains $launcher 'src-tauri\target\release\localflow.exe') -and (Test-Contains $launcher 'Start-Process -FilePath $resolvedReleaseExe')) {
    Add-Check "Launcher starts packaged release exe" "PASS" $releaseExe
  } else {
    Add-Check "Launcher starts packaged release exe" "FAIL" "Release exe launch command not found."
  }

  if ((Test-Contains $launcher "Signal-DesktopLaunch") -and (Test-Contains $launcher "desktop-launch-signal.json")) {
    Add-Check "Launcher writes desktop launch signal" "PASS" "desktop-launch-signal.json"
  } else {
    Add-Check "Launcher writes desktop launch signal" "FAIL" "Desktop launch signal write was not found."
  }

  if (Test-Contains $launcher "--localflow-desktop-launch") {
    Add-Check "Launcher passes desktop launch flag" "PASS" "--localflow-desktop-launch"
  } else {
    Add-Check "Launcher passes desktop launch flag" "FAIL" "Desktop launch flag was not found."
  }

  if ((Test-Contains $launcher "Stop-KnownViteServer") -and (Test-Contains $launcher "Get-NetTCPConnection -LocalPort 1420")) {
    Add-Check "Launcher stops repo Vite server" "PASS" "Stop-KnownViteServer checks port 1420"
  } else {
    Add-Check "Launcher stops repo Vite server" "FAIL" "Vite stop logic for port 1420 was not found."
  }

  if ((-not (Test-Contains $launcher "npm run dev")) -and (-not (Test-Contains $launcher "tauri dev"))) {
    Add-Check "Launcher does not start dev server" "PASS" "No npm run dev or tauri dev command"
  } else {
    Add-Check "Launcher does not start dev server" "FAIL" "Dev server command found in Start-LocalFlow.ps1."
  }
}

function Test-TauriDesktopConfig {
  if (-not (Test-Path -LiteralPath $tauriConfigPath)) {
    Add-Check "Tauri config exists" "FAIL" "Missing $tauriConfigPath"
    return
  }

  $config = Get-Content -Raw -LiteralPath $tauriConfigPath | ConvertFrom-Json
  $mainWindow = $config.app.windows | Where-Object { $_.label -eq "main" } | Select-Object -First 1
  $overlayWindow = $config.app.windows | Where-Object { $_.label -eq "overlay" } | Select-Object -First 1

  if ($mainWindow -and $mainWindow.visible -eq $false) {
    Add-Check "Main settings window starts hidden" "PASS" "main.visible=false"
  } else {
    Add-Check "Main settings window starts hidden" "FAIL" "main.visible is not false."
  }

  if ($overlayWindow -and $overlayWindow.url -eq "index.html?view=overlay" -and $overlayWindow.visible -eq $false) {
    Add-Check "Overlay window uses desktop overlay route" "PASS" "index.html?view=overlay"
  } else {
    Add-Check "Overlay window uses desktop overlay route" "FAIL" "Overlay route or visibility changed."
  }

  if ($overlayWindow -and $overlayWindow.skipTaskbar -eq $true -and $overlayWindow.focusable -eq $false) {
    Add-Check "Overlay remains desktop overlay-only chrome" "PASS" "skipTaskbar=true, focusable=false"
  } else {
    Add-Check "Overlay remains desktop overlay-only chrome" "FAIL" "Overlay taskbar/focus behavior changed."
  }

  $mainTsx = Get-Content -Raw -LiteralPath $mainTsxPath
  if ((Test-Contains $mainTsx 'view === "mobile-sdk-example"') -and (Test-Contains $mainTsx 'isOverlayView ? <VoiceOverlay /> : isMobileSdkExampleView ? <MobileSdkExample /> : <App />')) {
    Add-Check "Mobile SDK example is query-gated" "PASS" "?view=mobile-sdk-example only"
  } else {
    Add-Check "Mobile SDK example is query-gated" "FAIL" "Main route no longer keeps SDK example separate."
  }
}

function Test-HotkeyRegistration {
  if (-not (Test-Path -LiteralPath $hotkeysPath)) {
    Add-Check "Hotkey source exists" "FAIL" "Missing $hotkeysPath"
    return
  }

  $hotkeys = Get-Content -Raw -LiteralPath $hotkeysPath
  $lib = Get-Content -Raw -LiteralPath $libPath

  if ((Test-Contains $hotkeys "Modifiers::CONTROL | Modifiers::ALT") -and (Test-Contains $hotkeys "Code::Space") -and (Test-Contains $hotkeys "Ctrl+Alt+Space")) {
    Add-Check "Primary desktop hotkey remains Ctrl+Alt+Space" "PASS" "CONTROL + ALT + SPACE"
  } else {
    Add-Check "Primary desktop hotkey remains Ctrl+Alt+Space" "FAIL" "Primary hotkey registration changed."
  }

  if ((Test-Contains $hotkeys "Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT") -and (Test-Contains $hotkeys "Ctrl+Alt+Shift+Space")) {
    Add-Check "Fallback desktop hotkey remains Ctrl+Alt+Shift+Space" "PASS" "CONTROL + ALT + SHIFT + SPACE"
  } else {
    Add-Check "Fallback desktop hotkey remains Ctrl+Alt+Shift+Space" "FAIL" "Fallback hotkey registration changed."
  }

  if ((Test-Contains $lib "#[cfg(desktop)]") -and (Test-Contains $lib "hotkeys::register_default_hotkey(app.handle())")) {
    Add-Check "Desktop setup registers global hotkeys" "PASS" "register_default_hotkey in Tauri setup"
  } else {
    Add-Check "Desktop setup registers global hotkeys" "FAIL" "Tauri setup no longer registers desktop hotkeys."
  }

  if (-not (Test-Contains $hotkeys "trying fallback")) {
    Add-Check "Desktop hotkeys are not fallback-only" "PASS" "Both shortcuts are attempted independently"
  } else {
    Add-Check "Desktop hotkeys are not fallback-only" "FAIL" "Fallback-only registration text remains."
  }
}

function Test-RuntimeState {
  if (Test-Path -LiteralPath $releaseExe) {
    Add-Check "Packaged release exe exists" "PASS" (Resolve-ExistingPath $releaseExe)
  } else {
    Add-Check "Packaged release exe exists" "FAIL" "Missing $releaseExe"
  }

  $resolvedReleaseExe = Resolve-ExistingPath $releaseExe
  $releaseProcesses = @(
    Get-Process -Name localflow -ErrorAction SilentlyContinue |
      Where-Object { (Get-ProcessPath $_) -ieq $resolvedReleaseExe }
  )

  if ($releaseProcesses.Count -gt 0) {
    $processList = ($releaseProcesses | ForEach-Object { "PID $($_.Id): $(Get-ProcessPath $_)" }) -join "; "
    Add-Check "Release app process is running" "PASS" $processList
  } elseif ($RequireRunning) {
    Add-Check "Release app process is running" "FAIL" "No running localflow.exe from $resolvedReleaseExe"
  } else {
    Add-Check "Release app process is running" "WARN" "Not running. Use scripts\Start-LocalFlow.ps1 -Restart to launch it."
  }

  $portListeners = @(Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)
  if ($portListeners.Count -eq 0) {
    Add-Check "Port 1420 is not listening" "PASS" "No Vite dev server listener"
  } else {
    $listenerList = ($portListeners | ForEach-Object { "PID $($_.OwningProcess)" }) -join "; "
    Add-Check "Port 1420 is not listening" "FAIL" $listenerList
  }

  $viteProcesses = @(
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
      Where-Object {
        $commandLine = [string]$_.CommandLine
        $commandLine.Contains($repoRoot) -and
        ($commandLine -match "\bvite\b" -or $commandLine -match "node_modules[\\/]\.bin[\\/]vite")
      }
  )

  if ($viteProcesses.Count -eq 0) {
    Add-Check "No repo Vite process is running" "PASS" "No Vite process from $repoRoot"
  } else {
    $viteList = ($viteProcesses | ForEach-Object { "PID $($_.ProcessId): $($_.CommandLine)" }) -join "; "
    Add-Check "No repo Vite process is running" "FAIL" $viteList
  }
}

function Test-DesktopHealth {
  $health = Read-DesktopHealth
  if (-not $health) {
    if ($RequireRunning) {
      Add-Check "Desktop health file exists" "FAIL" "Missing $desktopHealthPath"
    } else {
      Add-Check "Desktop health file exists" "WARN" "Missing $desktopHealthPath"
    }
    return
  }

  Add-Check "Desktop health file exists" "PASS" $desktopHealthPath

  $registeredHotkeys = @($health.registeredHotkeys | Where-Object { $_ })
  if ($registeredHotkeys.Count -eq 0) {
    Add-Check "Health reports registered hotkeys" "FAIL" "No registered hotkeys in desktop-health.json"
  } else {
    Add-Check "Health reports registered hotkeys" "PASS" ($registeredHotkeys -join ", ")
  }

  foreach ($shortcut in @("Ctrl+Alt+Space", "Ctrl+Alt+Shift+Space")) {
    if ($registeredHotkeys -contains $shortcut) {
      Add-Check "Health confirms $shortcut" "PASS" "Registered"
    } else {
      Add-Check "Health confirms $shortcut" "FAIL" "Not registered. Failed hotkeys: $($health.failedHotkeys | ConvertTo-Json -Compress)"
    }
  }

  $failedHotkeys = @($health.failedHotkeys | Where-Object { $_ })
  if ($failedHotkeys.Count -eq 0) {
    Add-Check "Health reports no failed hotkeys" "PASS" "None"
  } else {
    Add-Check "Health reports no failed hotkeys" "WARN" ($failedHotkeys | ConvertTo-Json -Compress)
  }

  if ($health.shortcutLaunchAt) {
    Add-Check "Health recorded desktop shortcut launch" "PASS" $health.shortcutLaunchAt
  } else {
    Add-Check "Health recorded desktop shortcut launch" "WARN" "No shortcut launch timestamp recorded yet."
  }

  if ($health.lastOverlayEvent) {
    Add-Check "Health recorded overlay event" "PASS" "$($health.lastOverlayEvent.phase) at $($health.lastOverlayEvent.at)"
  } else {
    Add-Check "Health recorded overlay event" "WARN" "No overlay event recorded yet."
  }

  if ($ExpectRecentHotkeySeconds -gt 0) {
    if ($health.lastHotkeyEvent) {
      Add-Check "Health recorded last hotkey event" "PASS" "$($health.lastHotkeyEvent.shortcut) $($health.lastHotkeyEvent.state) at $($health.lastHotkeyEvent.at)"
      Test-RecentTimestamp "Recent hotkey event" $health.lastHotkeyEvent.at $ExpectRecentHotkeySeconds
    } else {
      Add-Check "Health recorded last hotkey event" "FAIL" "No hotkey event recorded."
    }
  }
}

Test-ShortcutChain
Test-VbsLauncher
Test-PowerShellLauncher
Test-TauriDesktopConfig
Test-HotkeyRegistration
Test-RuntimeState
Test-DesktopHealth

$checks | Format-Table -AutoSize -Wrap

$failures = @($checks | Where-Object { $_.Status -eq "FAIL" })
$warnings = @($checks | Where-Object { $_.Status -eq "WARN" })

if ($failures.Count -gt 0) {
  Write-Error "Desktop boot smoke check failed with $($failures.Count) failure(s)."
  exit 1
}

if ($warnings.Count -gt 0) {
  Write-Warning "Desktop boot smoke check completed with $($warnings.Count) warning(s)."
}

Write-Host "Desktop boot smoke check passed."
