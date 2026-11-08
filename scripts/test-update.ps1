# Prepares a local end-to-end test of the auto-updater without a GitHub release.
#
# It builds the app twice with a throwaway signing key: the version in tauri.conf.json as the
# "installed" build, and the next patch version as the "update". Both point their updater at
# http://127.0.0.1:8765/latest.json. The update, its signature and latest.json land in
# <out>/serve; `python -m http.server 8765` in that folder plays the GitHub release.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts/test-update.ps1 [-Out <folder>] [-SkipBuild]
# Then:   1. close every Multi Claude (the Electron one too), install <out>/installed/*.exe, start it
#         2. in <out>/serve run:  python -m http.server 8765
#         3. after ~10 s the update bar appears; Install and restart ends in the next version
# Neither build nor its key is meant to be shipped: the key lives only under <out>.

param(
  [string]$Out = (Join-Path $env:LOCALAPPDATA 'Temp\mc-update-test'),
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$conf = Get-Content (Join-Path $repo 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$current = [version]$conf.version
$next = "{0}.{1}.{2}" -f $current.Major, $current.Minor, ($current.Build + 1)
$port = 8765
$endpoint = "http://127.0.0.1:$port/latest.json"

New-Item -ItemType Directory -Force -Path $Out, (Join-Path $Out 'installed'), (Join-Path $Out 'serve') | Out-Null

# 1. a throwaway key pair
$keyFile = Join-Path $Out 'test.key'
if (-not (Test-Path $keyFile)) {
  npx tauri signer generate -w $keyFile -p 'test' --ci --force | Out-Null
}
$pubkey = (Get-Content "$keyFile.pub" -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $keyFile -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = 'test'

function Build-Version([string]$version) {
  $override = @{
    version = $version
    plugins = @{ updater = @{
      endpoints = @($endpoint)
      pubkey = $pubkey
      dangerousInsecureTransportProtocol = $true
    } }
  } | ConvertTo-Json -Depth 5
  # a file, not an inline string: PowerShell strips the quotes of a JSON argument on the way out
  $overrideFile = Join-Path $Out "override-$version.json"
  Set-Content $overrideFile $override -Encoding UTF8
  Write-Host "== building $version (updater at $endpoint)"
  npx tauri build --config $overrideFile
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed for $version" }
}

$bundle = Join-Path $repo 'target\release\bundle\nsis'
if (-not $SkipBuild) {
  Build-Version $current.ToString()
  Copy-Item (Join-Path $bundle "Multi Claude_${current}_x64-setup.exe") (Join-Path $Out 'installed') -Force
  Build-Version $next
}

# 2. the "release": installer, its signature and latest.json
$installer = Join-Path $bundle "Multi Claude_${next}_x64-setup.exe"
$sig = (Get-Content "$installer.sig" -Raw).Trim()
Copy-Item $installer (Join-Path $Out 'serve') -Force
$fileName = [uri]::EscapeDataString((Split-Path -Leaf $installer))
$latest = [ordered]@{
  version = $next
  notes = "Local update test.`n`n### Added`n- A test release served from 127.0.0.1:$port.`n`n### Changed`n- Nothing else; this build only differs in its version."
  pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
  platforms = @{ 'windows-x86_64' = @{ signature = $sig; url = "http://127.0.0.1:$port/$fileName" } }
}
$latest | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Out 'serve\latest.json') -Encoding UTF8

Write-Host ""
Write-Host "Ready."
Write-Host "  installed build : $(Join-Path $Out "installed\Multi Claude_${current}_x64-setup.exe")"
Write-Host "  update to serve : $(Join-Path $Out 'serve')"
Write-Host ""
Write-Host "Next: close every Multi Claude, install the installed build and start it, then run"
Write-Host "  cd `"$(Join-Path $Out 'serve')`"; python -m http.server $port"
Write-Host "The update bar appears about 10 s after the app starts."
