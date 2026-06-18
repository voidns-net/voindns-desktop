<#
.SYNOPSIS
  Headless Windows installer for the voidns privileged service + CLI.

.DESCRIPTION
  The GUI's NSIS bundle registers the service via install-service.nsh; this is
  the same logic without the GUI, for scripted installs and the tier2 e2e CI
  (which exercises the real install path on Windows: copy binaries -> register
  the `voidns` Windows service -> start it). Run elevated.

    install-service.ps1 -Action install -SourceDir <dir-with-built-exes> [-InstallDir <dir>]
    install-service.ps1 -Action uninstall [-InstallDir <dir>]

  -SourceDir holds voidns-service.exe and voidns.exe (e.g. target\release).
  -InstallDir defaults to "$env:ProgramFiles\VoidNS".
#>
[CmdletBinding()]
param(
  [ValidateSet('install', 'uninstall')]
  [string]$Action = 'install',
  [string]$SourceDir,
  [string]$InstallDir = (Join-Path $env:ProgramFiles 'VoidNS')
)

$ErrorActionPreference = 'Stop'
$svcBin = Join-Path $InstallDir 'voidns-service.exe'
$cliBin = Join-Path $InstallDir 'voidns.exe'

function Stop-AndDelete {
  # Idempotent: ignore "service not found" so install/uninstall can re-run.
  & sc.exe stop voidns   2>&1 | Out-Null
  & sc.exe delete voidns 2>&1 | Out-Null
  Start-Sleep -Milliseconds 500
}

if ($Action -eq 'uninstall') {
  Stop-AndDelete
  Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
  Write-Host "voidns service removed."
  exit 0
}

if (-not $SourceDir) { throw "-SourceDir is required for install" }
$srcSvc = Join-Path $SourceDir 'voidns-service.exe'
$srcCli = Join-Path $SourceDir 'voidns.exe'
if (-not (Test-Path $srcSvc)) { throw "missing $srcSvc" }

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Stop-AndDelete
Copy-Item -Force $srcSvc $svcBin
if (Test-Path $srcCli) { Copy-Item -Force $srcCli $cliBin }

# Register the service. `binPath` includes the `run` subcommand (daemon).
& sc.exe create voidns binPath= "`"$svcBin`" run" start= auto DisplayName= "voidns" | Out-Host
& sc.exe description voidns "voidns local DoH proxy and DNS redirector" | Out-Host

# A LocalSystem service has no console, so the daemon's stdout is discarded.
# Point it at a log file via a per-service environment block (REG_MULTI_SZ under
# the service key, read by the SCM at start) so the service is diagnosable.
$logDir = Join-Path $env:ProgramData 'VoidNS'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir 'service.log'
$svcKey = 'HKLM:\SYSTEM\CurrentControlSet\Services\voidns'
New-ItemProperty -Path $svcKey -Name Environment -PropertyType MultiString -Force `
  -Value @("VOIDNS_LOG_FILE=$logFile", "RUST_LOG=info,voidns_core=debug,hickory_resolver=debug,hickory_proto=debug") |
  Out-Null

& sc.exe start voidns | Out-Host

Write-Host "voidns service installed to $InstallDir and started. Logs: $logFile"
