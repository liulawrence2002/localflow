$ErrorActionPreference = "Stop"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path -LiteralPath $cargoBin) -and (($env:Path -split ";") -notcontains $cargoBin)) {
  $env:Path = "$cargoBin;$env:Path"
}

npm run format
npm run lint
npm run test
npm run build

if (Get-Command cargo -ErrorAction SilentlyContinue) {
  Push-Location src-tauri
  cargo fmt --check
  # Release only: the debug profile's build cache grew to 13 GB on this project.
  cargo test --release --lib
  cargo build --release --examples
  Pop-Location
} else {
  Write-Warning "Skipping Rust checks because cargo is not on PATH."
}
