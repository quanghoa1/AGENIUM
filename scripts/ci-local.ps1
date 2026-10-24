# Local CI (GitHub Actions is not used): fmt, clippy, tests, generated-schema check, scripted golden tasks.
# Usage: .\scripts\ci-local.ps1 [-Quick] [-FailFast]     Logs: target\ci-local\<step>.log
param([switch]$Quick, [switch]$FailFast)
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$logs = Join-Path $root "target\ci-local"
New-Item -ItemType Directory -Force $logs | Out-Null
$steps = @(
  @{ name = "fmt";     cmd = "cargo fmt --all --check" },
  @{ name = "clippy";  cmd = "cargo clippy --workspace --all-targets -- -D warnings" },
  @{ name = "test";    cmd = "cargo test --workspace" },
  @{ name = "schemas"; cmd = "cargo xtask schemas --check" },
  @{ name = "vendor";  cmd = "cargo xtask vendor-check" }
)
if (-not $Quick) { $steps += @{ name = "golden-scripted"; cmd = "cargo run -q -p agenium-eval -- --out target\ci-local\eval" } }
$results = @()
foreach ($s in $steps) {
  $log = Join-Path $logs "$($s.name).log"
  $t = [Diagnostics.Stopwatch]::StartNew()
  cmd /c "$($s.cmd) > `"$log`" 2>&1"
  $ok = ($LASTEXITCODE -eq 0)
  $t.Stop()
  $results += [pscustomobject]@{ step = $s.name; result = $(if ($ok) { "PASS" } else { "FAIL" }); seconds = [math]::Round($t.Elapsed.TotalSeconds, 1); log = $log }
  if (-not $ok -and $FailFast) { break }
}
$results | Format-Table -AutoSize
if ($results.result -contains "FAIL") { exit 1 }
