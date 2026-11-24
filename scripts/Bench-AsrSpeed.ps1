param(
  # Threads: CPU thread sweep. Gpu: Vulkan GPU against the CPU, warm and cold. All: both.
  [ValidateSet("Threads", "Gpu", "All")]
  [string]$Part = "All",
  # Passes over the corpus for the warm (steady-state) runs.
  [int]$Passes = 3,
  # Thread counts for the sweep. 0 would mean the library default (logical CPUs capped at 8).
  [string]$SweepThreads = "4,6,8,12,22",
  # CPU threads for the CPU side of the GPU comparison (and for the few CPU ops on the GPU path).
  [int]$CpuThreads = 8,
  [string]$Model = "parakeet-tdt-0.6b-v3-Q5_K_M.gguf",
  [string]$OutDir = "docs\benchmarks",
  [string]$VulkanSdk = "C:\VulkanSDK\1.4.363.0"
)

# Speech-recognition speed benchmarks. Run on a quiet machine: no compiles, no other
# benchmarks, on AC power. Nothing leaves the machine and no transcript text is printed.
# Results are written as JSON under $OutDir; the console shows the summary tables.
#
#   .\scripts\Bench-AsrSpeed.ps1                 # both parts
#   .\scripts\Bench-AsrSpeed.ps1 -Part Threads   # CPU thread sweep only
#   .\scripts\Bench-AsrSpeed.ps1 -Part Gpu       # needs the Vulkan SDK to build
#
# The GPU part builds the bench with `--features gpu-vulkan`. That build is release-only and
# needs roughly 1 GB of extra disk for the Vulkan shaders the first time.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$tauri = Join-Path $root "src-tauri"
$runtime = Join-Path $root ".localflow-runtime"
$corpus = Join-Path $runtime "bench-corpus"
$out = Join-Path $root $OutDir
New-Item -ItemType Directory -Force $out | Out-Null

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$cmakeBin = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"
$env:PATH = "$cargoBin;$cmakeBin;$env:PATH"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  throw "cargo was not found in $cargoBin."
}
if (-not (Test-Path -LiteralPath (Join-Path $runtime "models\$Model"))) {
  throw "Missing $Model in $runtime\models. Run scripts\Get-AsrModel.ps1."
}

$stamp = Get-Date -Format "yyyyMMdd-HHmm"
$common = @("--runtime", $runtime, "--corpus", $corpus, "--concurrency", "1")

function Invoke-Bench {
  param([string[]]$CargoArgs, [string[]]$BenchArgs)
  Push-Location $tauri
  try {
    & cargo run --release @CargoArgs --example asr_bench -- @common @BenchArgs
    if ($LASTEXITCODE -ne 0) { throw "asr_bench failed with exit code $LASTEXITCODE" }
  } finally {
    Pop-Location
  }
}

if ($Part -in @("Threads", "All")) {
  Write-Host "== CPU thread sweep ($SweepThreads), $Passes passes, full corpus =="
  Invoke-Bench -CargoArgs @() -BenchArgs @(
    "--config", "inprocess:${Model}:1:0:cpu",
    "--sweep-threads", $SweepThreads,
    "--passes", "$Passes",
    "--idle-secs", "30",
    "--json", (Join-Path $out "asr-threads-$stamp.json"))
}

if ($Part -in @("Gpu", "All")) {
  if (-not (Test-Path -LiteralPath (Join-Path $VulkanSdk "Bin\glslc.exe"))) {
    throw "The Vulkan SDK was not found at $VulkanSdk (glslc.exe is needed to build the GPU backend)."
  }
  $env:VULKAN_SDK = $VulkanSdk
  $env:PATH = "$VulkanSdk\Bin;$env:PATH"
  # The Vulkan backend links vulkan-1.lib, which the linker finds only through LIB.
  $env:LIB = "$VulkanSdk\Lib;$env:LIB"
  $gpu = @("--features", "gpu-vulkan")
  $configs = @("--config", "inprocess:${Model}:1:${CpuThreads}:cpu", "--config", "inprocess:${Model}:1:${CpuThreads}:gpu")

  Write-Host "== GPU against CPU, warm, $Passes passes, full corpus =="
  Invoke-Bench -CargoArgs $gpu -BenchArgs (@("--list-devices") + $configs + @(
    "--passes", "$Passes", "--idle-secs", "30",
    "--json", (Join-Path $out "asr-gpu-warm-$stamp.json")))

  Write-Host "== GPU against CPU, phrase-sized clips (up to 3 s), warm =="
  Invoke-Bench -CargoArgs $gpu -BenchArgs ($configs + @(
    "--max-seconds", "3", "--passes", "$Passes",
    "--json", (Join-Path $out "asr-gpu-short-$stamp.json")))

  # Cold: no warm-up, so each config's first request pays its one-time costs (shader
  # compiles on the GPU). The NVIDIA driver keeps a shader cache on disk, so only the very
  # first run on a machine is fully cold; later processes start partly warm.
  Write-Host "== GPU against CPU, phrase-sized clips, cold (no warm-up) =="
  Invoke-Bench -CargoArgs $gpu -BenchArgs ($configs + @(
    "--max-seconds", "3", "--no-warm-up", "--passes", "1",
    "--json", (Join-Path $out "asr-gpu-cold-$stamp.json")))
}

Write-Host "Results written to $out (*-$stamp.json)."
