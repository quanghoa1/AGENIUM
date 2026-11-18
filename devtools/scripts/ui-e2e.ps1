# End-to-end UI test (Windows), run by hand: built and started as ui-smoke.ps1 does (debug build, Vite, a throwaway
# root, an in-memory vault key), but driven as a user with a real model: apps/desktop/ui-tests/e2e-work.mjs makes a
# project and a Solo agent on the Claude Code CLI, works a request to closed tickets and reads every screen that shows
# it. Uses the user's Claude Code login (a few Haiku turns).
# Usage: .\devtools\scripts\ui-e2e.ps1 [-Port 9241]      Screenshots and logs: target\ci-local\ui-e2e\
param([int]$Port = 9241)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$out = Join-Path $root "target\ci-local\ui-e2e"
New-Item -ItemType Directory -Force $out | Out-Null
$desktop = Join-Path $root "apps\desktop"

Push-Location $desktop
try {
  cargo build -q
  if ($LASTEXITCODE -ne 0) { throw "desktop build failed" }
} finally { Pop-Location }
if (-not (Test-Path (Join-Path $desktop "ui\node_modules"))) { npm --prefix (Join-Path $desktop "ui") ci | Out-Null }

$home_ = Join-Path ([IO.Path]::GetTempPath()) ("agenium-ui-e2e-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$cache = Join-Path $root "target\ci-local\model-cache"
if (-not (Test-Path (Join-Path $cache "models\bge-small-en-v1.5\model.safetensors"))) {
  cargo run -q -p agenium-cli -- --root $cache models download
  if ($LASTEXITCODE -ne 0) { throw "the embedding model could not be downloaded" }
}
New-Item -ItemType Directory -Force (Join-Path $home_ "models") | Out-Null
Copy-Item -Recurse (Join-Path $cache "models\bge-small-en-v1.5") (Join-Path $home_ "models")
$vite = $null; $app = $null; $code = 1
try {
  $served = $false
  try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "http://127.0.0.1:5173/" | Out-Null; $served = $true } catch {}
  if (-not $served) {
    $vite = Start-Process -PassThru -WindowStyle Hidden -FilePath "cmd.exe" `
      -ArgumentList "/c npm --prefix `"$desktop\ui`" run dev > `"$out\vite.log`" 2>&1"
  }
  $ready = $false
  for ($i = 0; $i -lt 60 -and -not $ready; $i++) {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "http://127.0.0.1:5173/" | Out-Null; $ready = $true } catch { Start-Sleep 1 }
  }
  if (-not $ready) { throw "Vite did not start (see $out\vite.log)" }

  $env:AGENIUM_HOME = $home_
  $env:AGENIUM_UI_TEST_KEYRING = "memory"
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
  $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $home_ "webview2"
  $app = Start-Process -PassThru -FilePath (Join-Path $desktop "target\debug\agenium-desktop.exe") `
    -RedirectStandardOutput "$out\app.log" -RedirectStandardError "$out\app.err.log"
  node (Join-Path $desktop "ui-tests\e2e-work.mjs") $out $Port | Tee-Object -FilePath "$out\result.txt"
  $code = $LASTEXITCODE
} finally {
  Remove-Item Env:AGENIUM_HOME, Env:AGENIUM_UI_TEST_KEYRING, Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, Env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  if ($vite) { try { & taskkill /PID $vite.Id /T /F 2>&1 | Out-Null } catch {} }
  Start-Sleep 1
  Remove-Item -Recurse -Force $home_ -ErrorAction SilentlyContinue
}
exit $code
