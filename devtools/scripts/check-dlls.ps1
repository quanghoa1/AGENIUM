# The Windows binaries must start on a clean Windows: none may import a DLL of the MSYS2 GNU toolchain (libstdc++-6,
# libgcc_s_seh-1, libwinpthread-1), which only a development machine has on its PATH. Run by ci-local.ps1.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build -q -p agenium-cli
if ($LASTEXITCODE -ne 0) { exit 1 }
$imports = objdump -p target\debug\agenium.exe | Select-String "DLL Name:" | ForEach-Object { ($_.Line -split ":")[1].Trim().ToLower() } | Sort-Object -Unique
$gnu = $imports | Where-Object { $_ -match "^lib(stdc\+\+|gcc_s|winpthread)" }
if ($gnu) {
  "agenium.exe imports $($gnu -join ', '): link them statically (.cargo/config.toml)"
  exit 1
}
"agenium.exe imports only system DLLs: $($imports -join ', ')"
