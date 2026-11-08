# Builds src-tauri/resources/explorer/mc_explorer_command_x64.dll, the File Explorer context menu
# command (crates/mc-explorer-command), and checks the result: an x64 DLL that exports exactly
# DllCanUnloadNow and DllGetClassObject and imports only DLLs that ship with Windows.
#
# The built DLL is committed, so ordinary app builds do not need this. Run it with
# `npm run build:shell` after changing the crate, and bump Version in AppxManifest.xml.
# Needs the MSVC Rust toolchain and Visual Studio (Build Tools) for dumpbin.exe.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
$output = Join-Path $root 'src-tauri\resources\explorer\mc_explorer_command_x64.dll'

# Everything the DLL may import ships with Windows (Rust's std adds ntdll and the api-ms-win-* sets).
$allowedImports = '^(kernel32|user32|ole32|oleaut32|combase|shell32|shlwapi|ntdll|advapi32|bcrypt|bcryptprimitives|ucrtbase|api-ms-win-(crt|core)-[a-z0-9-]+)\.dll$'
$expectedExports = @('DllCanUnloadNow', 'DllGetClassObject')

if ($env:OS -ne 'Windows_NT') {
  Write-Error 'The Explorer context menu DLL is built on Windows only.'
}

function Find-Dumpbin {
  $onPath = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (Test-Path -LiteralPath $vswhere) {
    $found = & $vswhere -latest -products * -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' | Select-Object -Last 1
    if ($found) { return $found }
  }
  foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
    if (-not $base) { continue }
    $found = Get-ChildItem -Path (Join-Path $base 'Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') -ErrorAction SilentlyContinue |
      Sort-Object FullName | Select-Object -Last 1
    if ($found) { return $found.FullName }
  }
  Write-Error 'dumpbin.exe was not found; install Visual Studio Build Tools with the C++ workload.'
}

Push-Location $root
try {
  & cargo build --release -p mc-explorer-command
  if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed ($LASTEXITCODE)" }
  $targetDir = (& cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
  if ($LASTEXITCODE -ne 0 -or -not $targetDir) { Write-Error 'cargo metadata failed' }
} finally {
  Pop-Location
}

$built = Join-Path $targetDir 'release\mc_explorer_command.dll'
Copy-Item -LiteralPath $built -Destination $output -Force

$dumpbin = Find-Dumpbin
$headers = & $dumpbin /nologo /headers $output
$exportLines = & $dumpbin /nologo /exports $output
$dependentLines = & $dumpbin /nologo /dependents $output

$machine = ($headers | Select-String -Pattern '^\s*([0-9A-Fa-f]+) machine' | Select-Object -First 1)
$machine = if ($machine) { $machine.Matches[0].Groups[1].Value } else { '?' }

# "    ordinal hint RVA      name" rows: "          1    0 00001000 DllCanUnloadNow = ..."
$exports = @($exportLines | Select-String -Pattern '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]{8}\s+(\S+)' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Sort-Object)

# the lines between "Image has the following dependencies:" and "Summary"
$imports = @()
$inList = $false
foreach ($line in $dependentLines) {
  if ($line -match 'Image has the following dependencies') { $inList = $true; continue }
  if ($line -match '^\s*Summary') { break }
  if ($inList -and $line.Trim()) { $imports += $line.Trim() }
}
$imports = @($imports | Sort-Object -Unique)

$problems = @()
if ($machine -ne '8664') { $problems += "machine is 0x$machine, expected x64 (0x8664)" }
if (($exports -join ',') -ne ($expectedExports -join ',')) { $problems += "exports are [$($exports -join ', ')], expected [$($expectedExports -join ', ')]" }
$foreign = @($imports | Where-Object { $_ -notmatch $allowedImports })
if ($foreign.Count -gt 0) { $problems += "imports DLLs that are not part of Windows: $($foreign -join ', ')" }

Write-Host "$((Resolve-Path -LiteralPath $output -Relative)): $((Get-Item -LiteralPath $output).Length) bytes, machine 0x$machine"
Write-Host "  exports: $($exports -join ', ')"
Write-Host "  imports: $($imports -join ', ')"
if ($problems.Count -gt 0) {
  foreach ($problem in $problems) { Write-Host "  ERROR: $problem" -ForegroundColor Red }
  exit 1
}
