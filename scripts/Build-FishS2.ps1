<#
.SYNOPSIS
  Builds the local Fish S2 Pro voice engine (s2.cpp) into a LocalFlow runtime folder (fish).

.DESCRIPTION
  LocalFlow's read-aloud feature runs Fish Audio S2 Pro through s2.cpp, a community ggml
  runtime (https://github.com/rodrigomatta/s2.cpp). There is no prebuilt Windows binary, so
  this script compiles a pinned commit from source with the CMake that ships with the Visual
  Studio Build Tools. This is an explicit, user-run step; LocalFlow never downloads or builds
  anything by itself.

  Backends, with speeds measured on a Core Ultra 7 165H and an RTX 4070 Laptop (8 GB), q4_k_m:
    cpu     No extra software. About 11 times slower than real time.
    cuda    NVIDIA GPUs. About 2.8 times slower than real time: the fastest option measured.
            Needs the CUDA Toolkit 12.4 or later: either installed (CUDA_PATH set) or just its
            nvcc, cudart, cublas, and cccl component folders merged into
            .localflow-runtime\build\cuda (they can be extracted from NVIDIA's installer
            without running it). Built with Ninja, so no Visual Studio integration is needed.
    vulkan  Needs the Vulkan SDK (VULKAN_SDK set). Measured SLOWER than cpu here (about 17
            times slower than real time, plus a long start-up), so it is not recommended.

  s2.cpp is alpha software. The S2 Pro weights are under the Fish Audio Research License:
  research and personal non-commercial use only.

  The build runs in a fresh folder of your own (-BuildDir), which is emptied first and
  removed afterwards unless -KeepBuildTree is given. Only s2.exe and the ggml DLLs the
  chosen backend needs are copied to <Destination>\fish.

.PARAMETER Destination
  The runtime folder. Default: the checkout's .localflow-runtime when this script is in a Git
  checkout, otherwise %LOCALAPPDATA%\LocalFlow, which an installed LocalFlow reads. Download
  the model (Get-FishModel.ps1) into the same folder.

.PARAMETER BuildDir
  Where to compile. Default: lf-s2-<backend> in your user profile. It must not exist, or must
  be owned by you. The Vulkan backend needs short paths: if its build fails on path length,
  pass a shorter folder that you own.

.EXAMPLE
  .\scripts\Build-FishS2.ps1                  # CPU build
  .\scripts\Build-FishS2.ps1 -Backend cuda    # NVIDIA GPU build
#>
param(
  [ValidateSet("cpu", "vulkan", "cuda")]
  [string]$Backend = "cpu",
  [string]$Destination,
  [string]$BuildDir,
  # Commit of rodrigomatta/s2.cpp this integration was written against.
  [string]$Commit = "2c33261938da1a41d713768b1b391b4d368d7d2c",
  [switch]$KeepBuildTree
)

$ErrorActionPreference = "Stop"

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
$runtime = $Destination
$source = Join-Path $runtime "build\s2.cpp"
# The Vulkan backend builds a nested shader-generator project whose paths exceed the Windows
# 260-character limit when the build tree sits under the repository, so the default build
# folder is short and directly in the user profile. It is never at the drive root, where
# any local account can create folders: a folder planted there could feed the build.
if (-not $BuildDir) { $BuildDir = Join-Path $env:USERPROFILE "lf-s2-$Backend" }
$buildDir = [IO.Path]::GetFullPath($BuildDir)

function Assert-OwnedByMe([string]$Path) {
  $me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
  $owner = (Get-Acl -LiteralPath $Path).Owner
  if ($owner -ne $me) {
    throw "$Path already exists and is owned by $owner, not $me. Delete it or pass another -BuildDir."
  }
}
$installDir = Join-Path $runtime "fish"

function Find-CMake {
  $onPath = Get-Command cmake -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }

  $roots = @(
    "${env:ProgramFiles(x86)}\Microsoft Visual Studio",
    "$env:ProgramFiles\Microsoft Visual Studio"
  )
  foreach ($root in $roots) {
    if (-not (Test-Path -LiteralPath $root)) { continue }
    $found = Get-ChildItem -LiteralPath $root -Recurse -Filter cmake.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -like "*CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" } |
      Select-Object -First 1
    if ($found) { return $found.FullName }
  }
  throw "CMake was not found. Install the Visual Studio Build Tools with the C++ CMake tools (scripts\Install-Prereqs.ps1 -Install covers the Build Tools)."
}

if ($Backend -eq "vulkan" -and -not $env:VULKAN_SDK) {
  throw "The Vulkan backend needs the Vulkan SDK. Install it from https://vulkan.lunarg.com/sdk/home (or 'winget install KhronosGroup.VulkanSDK'), open a new terminal so VULKAN_SDK is set, and run this script again."
}
$cudaRoot = $null
if ($Backend -eq "cuda") {
  $candidates = @((Join-Path $runtime "build\cuda"), $env:CUDA_PATH) | Where-Object { $_ }
  $cudaRoot = $candidates | Where-Object { Test-Path -LiteralPath (Join-Path $_ "bin\nvcc.exe") } | Select-Object -First 1
  if (-not $cudaRoot) {
    throw "The CUDA backend needs the NVIDIA CUDA Toolkit 12.4 or later (nvcc, cudart, cublas, cccl). Install it from https://developer.nvidia.com/cuda-downloads so CUDA_PATH is set, or place those components in $(Join-Path $runtime 'build\cuda'), then run this script again."
  }
}

function Find-VsFile([string]$pattern) {
  foreach ($root in @("${env:ProgramFiles(x86)}\Microsoft Visual Studio", "$env:ProgramFiles\Microsoft Visual Studio")) {
    if (-not (Test-Path -LiteralPath $root)) { continue }
    $found = Get-ChildItem -LiteralPath $root -Recurse -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -like $pattern } | Select-Object -First 1
    if ($found) { return $found.FullName }
  }
  return $null
}

$cmake = Find-CMake
Write-Host "Using CMake: $cmake"

# Start from an empty build folder, so nothing left in it can end up in the engine.
if (Test-Path -LiteralPath $buildDir) {
  Assert-OwnedByMe $buildDir
  Remove-Item -LiteralPath $buildDir -Recurse -Force
}
New-Item -ItemType Directory -Path $buildDir | Out-Null

if (-not (Test-Path -LiteralPath (Join-Path $source ".git"))) {
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $source) | Out-Null
  git clone --recurse-submodules https://github.com/rodrigomatta/s2.cpp.git $source
  if ($LASTEXITCODE -ne 0) { throw "git clone failed." }
}

Push-Location $source
try {
  $head = (git rev-parse HEAD).Trim()
  if ($head -ne $Commit) {
    git fetch --quiet origin $Commit
    if ($LASTEXITCODE -ne 0) { throw "Could not fetch commit $Commit." }
    git checkout --quiet $Commit
    if ($LASTEXITCODE -ne 0) { throw "Could not check out commit $Commit." }
    git submodule update --init --recursive --quiet
    if ($LASTEXITCODE -ne 0) { throw "Could not update submodules." }
  }
  Write-Host "s2.cpp at $((git rev-parse --short HEAD).Trim())"

  if ($Backend -eq "cuda") {
    # CUDA is built with Ninja inside the MSVC environment. The Visual Studio generator would
    # need NVIDIA's Visual Studio integration installed; Ninja only needs nvcc on disk.
    $vcvars = Find-VsFile "*\VC\Auxiliary\Build\vcvars64.bat"
    $ninja = Find-VsFile "*\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe"
    if (-not $vcvars -or -not $ninja) {
      throw "The CUDA build needs the Visual Studio Build Tools with the C++ CMake tools (vcvars64.bat and ninja.exe were not both found)."
    }
    # /D_MBCS matches what the Visual Studio generator defines; without it the engine's
    # bundled filesystem shim fails to compile under Ninja.
    $batch = @"
@echo off
call "$vcvars" >nul 2>nul
if errorlevel 1 exit /b 1
set "PATH=$(Split-Path -Parent $cmake);$(Split-Path -Parent $ninja);$cudaRoot\bin;%PATH%"
set "CUDA_PATH=$cudaRoot"
cmake -S "$source" -B "$buildDir" -G Ninja -DCMAKE_BUILD_TYPE=Release -DS2_CUDA=ON -DCUDAToolkit_ROOT="$cudaRoot" -DCMAKE_CUDA_COMPILER="$cudaRoot\bin\nvcc.exe" -DCMAKE_CXX_FLAGS="/D_MBCS /DWIN32 /D_WINDOWS /EHsc /GR" 2>&1
if errorlevel 1 exit /b 1
cmake --build "$buildDir" --parallel 2>&1
"@
    $batchPath = Join-Path ([IO.Path]::GetTempPath()) ("localflow-build-s2-cuda-" + [Guid]::NewGuid().ToString("N") + ".cmd")
    Set-Content -LiteralPath $batchPath -Value $batch -Encoding ascii
    & cmd.exe /c "`"$batchPath`""
    $buildExit = $LASTEXITCODE
    Remove-Item -LiteralPath $batchPath -ErrorAction SilentlyContinue
    if ($buildExit -ne 0) { throw "CUDA build failed." }
  }
  else {
    $configure = @("-B", $buildDir, "-DCMAKE_BUILD_TYPE=Release")
    if ($Backend -eq "vulkan") { $configure += "-DS2_VULKAN=ON" }

    & $cmake @configure
    if ($LASTEXITCODE -ne 0) { throw "CMake configure failed." }

    & $cmake --build $buildDir --config Release --parallel
    if ($LASTEXITCODE -ne 0) { throw "CMake build failed." }
  }
}
finally {
  Pop-Location
}

# The Visual Studio generator puts the program in Release\, Ninja in the build folder itself.
$exe = @("Release\s2.exe", "s2.exe") |
  ForEach-Object { Join-Path $buildDir $_ } |
  Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
  Select-Object -First 1
if (-not $exe) { throw "The build finished but s2.exe is not in $buildDir\Release or $buildDir." }

# The ggml libraries a shared-library build produces for this backend, each exactly once.
$backendDll = @{ cpu = $null; vulkan = "ggml-vulkan.dll"; cuda = "ggml-cuda.dll" }[$Backend]
$expectedDlls = @("ggml.dll", "ggml-base.dll", "ggml-cpu.dll", $backendDll) | Where-Object { $_ }
$dlls = @()
foreach ($name in $expectedDlls) {
  $found = @(Get-ChildItem -LiteralPath $buildDir -Recurse -File -Filter $name)
  if ($found.Count -gt 1) { throw "The build produced more than one $name; not installing." }
  if ($found.Count -eq 1) { $dlls += $found[0].FullName }
}
if ($backendDll -and -not ($dlls | Where-Object { (Split-Path -Leaf $_) -eq $backendDll })) {
  throw "The build finished but $backendDll was not produced."
}

# Replace any previous engine so a stale binary or DLL from another backend cannot linger.
if (Test-Path -LiteralPath $installDir) { Remove-Item -LiteralPath $installDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

Copy-Item -LiteralPath $exe -Destination $installDir
# Shared-library builds place the ggml DLLs elsewhere in the tree; the engine needs them
# beside it.
foreach ($dll in $dlls) { Copy-Item -LiteralPath $dll -Destination $installDir -Force }
if ($Backend -eq "cuda") {
  # The engine loads the CUDA runtime and cuBLAS at start; ship them beside it so it runs
  # without a system-wide CUDA install.
  Get-ChildItem -LiteralPath (Join-Path $cudaRoot "bin") -Filter *.dll |
    Where-Object { $_.Name -match "^(cudart64|cublas64|cublasLt64)_\d+\.dll$" } |
    ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $installDir -Force }
}
Copy-Item -LiteralPath (Join-Path $source "LICENSE.md") -Destination (Join-Path $installDir "LICENSE-fish-audio-research.md")
Set-Content -LiteralPath (Join-Path $installDir "backend.txt") -Value $Backend -Encoding ascii

if (-not $KeepBuildTree) {
  # The build tree is hundreds of megabytes and is not needed at run time.
  Remove-Item -LiteralPath $buildDir -Recurse -Force
}

$sizeMb = [math]::Round(((Get-ChildItem -LiteralPath $installDir -Recurse | Measure-Object Length -Sum).Sum) / 1MB, 1)
Write-Host ""
Write-Host "Installed the Fish S2 engine ($Backend) to $installDir ($sizeMb MB)."
Write-Host "Next: .\scripts\Get-FishModel.ps1 if the model is not downloaded yet,"
Write-Host "then use the tray item 'Read last transcript aloud'."
