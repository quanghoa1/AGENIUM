# UI smoke test (Windows): builds the desktop app (debug), serves the UI with Vite, starts the app on a throwaway root
# with an in-memory vault key (nothing reaches the OS keyring) and drives it through the WebView2 DevTools port with
# apps/desktop/ui-tests/smoke.mjs (Node 22+). No model is called.
# Usage: .\devtools\scripts\ui-smoke.ps1 [-Port 9222]      Screenshots and logs: target\ci-local\ui-smoke\
param([int]$Port = 9222)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$out = Join-Path $root "target\ci-local\ui-smoke"
New-Item -ItemType Directory -Force $out | Out-Null
$desktop = Join-Path $root "apps\desktop"

Push-Location $desktop
try {
  cargo build -q
  if ($LASTEXITCODE -ne 0) { throw "desktop build failed" }
} finally { Pop-Location }
if (-not (Test-Path (Join-Path $desktop "ui\node_modules"))) { npm --prefix (Join-Path $desktop "ui") ci | Out-Null }

$home_ = Join-Path ([IO.Path]::GetTempPath()) ("agenium-ui-smoke-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$vite = $null; $app = $null
try {
  $vite = Start-Process -PassThru -WindowStyle Hidden -FilePath "cmd.exe" `
    -ArgumentList "/c npm --prefix `"$desktop\ui`" run dev > `"$out\vite.log`" 2>&1"
  $ready = $false
  for ($i = 0; $i -lt 60 -and -not $ready; $i++) {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "http://127.0.0.1:5173/" | Out-Null; $ready = $true } catch { Start-Sleep 1 }
  }
  if (-not $ready) { throw "Vite did not start (see $out\vite.log)" }

  $env:AGENIUM_HOME = $home_
  $env:AGENIUM_UI_TEST_KEYRING = "memory"
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
  $app = Start-Process -PassThru -FilePath (Join-Path $desktop "target\debug\agenium-desktop.exe") `
    -RedirectStandardOutput "$out\app.log" -RedirectStandardError "$out\app.err.log"
  node (Join-Path $desktop "ui-tests\smoke.mjs") $out $Port
  $code = $LASTEXITCODE
} finally {
  Remove-Item Env:AGENIUM_HOME, Env:AGENIUM_UI_TEST_KEYRING, Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  if ($vite) { taskkill /PID $vite.Id /T /F 2>$null | Out-Null }
  Start-Sleep 1
  Remove-Item -Recurse -Force $home_ -ErrorAction SilentlyContinue
}
exit $code
