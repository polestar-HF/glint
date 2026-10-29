<#
.SYNOPSIS
  Copy the Visual C++ runtime DLLs for app-local deployment into src-tauri/resources/vcrt/.

.DESCRIPTION
  onnxruntime.dll and glint.exe import MSVCP140.dll, MSVCP140_1.dll,
  VCRUNTIME140.dll and VCRUNTIME140_1.dll. A clean Windows machine may lack the
  VC++ Redistributable, so Glint ships these four DLLs next to the exe
  (Microsoft permits app-local deployment of the redistributable DLLs).

  The DLLs come from the official VS redistributable folder of the installed
  Visual Studio / Build Tools (x64\Microsoft.VC*.CRT). Each copy is checked:
  at least 10 KB, valid Authenticode signature, signed by Microsoft Corporation.
  File names are normalized to lowercase. Nothing is written unless all four
  pass. Release builds must only ever use DLLs copied by this script on a
  Windows machine; the Linux helper fetch-vcrt.sh writes empty placeholders.

  The folder is located, in order, from: -CrtDir, $env:VCToolsRedistDir (set in
  a VS developer shell), -VsInstallPath, vswhere.

  Windows PowerShell 5.1 or PowerShell 7+, on Windows only.

.PARAMETER Dest
  Output folder. Default: src-tauri/resources/vcrt

.PARAMETER CrtDir
  Use this folder (the one that contains vcruntime140.dll) directly.

.PARAMETER VsInstallPath
  Visual Studio installation folder; skips vswhere.
#>
[CmdletBinding()]
param(
  [string]$Dest,
  [string]$CrtDir,
  [string]$VsInstallPath
)

$ErrorActionPreference = 'Stop'

$Names = @('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
$MinBytes = 10240
$Marker = 'PLACEHOLDER-NOT-SHIPPABLE.txt' # written by fetch-vcrt.sh

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Dest) { $Dest = Join-Path $RepoRoot 'src-tauri/resources/vcrt' }
$Dest = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Dest)

function Write-Log([string]$Message) { Write-Host "fetch-vcrt: $Message" }

# The x64 CRT folder (holds vcruntime140.dll) under a VS redist version folder.
function Find-CrtUnder([string]$VersionDir) {
  $x64 = Join-Path $VersionDir 'x64'
  if (-not (Test-Path -LiteralPath $x64 -PathType Container)) { return $null }
  $crt = Get-ChildItem -LiteralPath $x64 -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'vcruntime140.dll') } |
    Select-Object -First 1
  if ($crt) { return $crt.FullName }
  return $null
}

function Find-CrtDir {
  if ($CrtDir) { return $CrtDir }

  if ($env:VCToolsRedistDir) {
    $found = Find-CrtUnder ($env:VCToolsRedistDir.TrimEnd('\', '/'))
    if ($found) { return $found }
  }

  $vs = $VsInstallPath
  if (-not $vs -and ${env:ProgramFiles(x86)}) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (Test-Path -LiteralPath $vswhere) {
      $found = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
      if ($LASTEXITCODE -eq 0 -and $found.Count -gt 0) { $vs = [string]$found[0] }
    }
  }
  if (-not $vs) { throw 'fetch-vcrt: no Visual Studio with the C++ tools found (set VCToolsRedistDir, or pass -CrtDir / -VsInstallPath)' }

  $base = Join-Path $vs 'VC/Redist/MSVC'
  $candidates = @()
  # The version the installed toolset was built with comes first.
  $versionFile = Join-Path $vs 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt'
  if (Test-Path -LiteralPath $versionFile) {
    $candidates += Join-Path $base ((Get-Content -LiteralPath $versionFile -Raw).Trim())
  }
  if (Test-Path -LiteralPath $base) {
    $candidates += Get-ChildItem -LiteralPath $base -Directory |
      Where-Object { $_.Name -match '^\d+\.\d+\.\d+' } |
      Sort-Object { [version]$_.Name } -Descending |
      ForEach-Object { $_.FullName }
  }
  foreach ($candidate in $candidates) {
    $found = Find-CrtUnder $candidate
    if ($found) { return $found }
  }
  throw "fetch-vcrt: no x64 Microsoft.VC*.CRT redistributable folder under $base"
}

function Assert-MicrosoftSigned([string]$Path) {
  $name = Split-Path -Leaf $Path
  if ((Get-Item -LiteralPath $Path).Length -lt $MinBytes) {
    throw "fetch-vcrt: $name is smaller than $MinBytes bytes"
  }
  $sig = Get-AuthenticodeSignature -LiteralPath $Path
  if ($sig.Status -ne 'Valid') {
    throw "fetch-vcrt: $name has signature status '$($sig.Status)', expected Valid"
  }
  $subject = [string]$sig.SignerCertificate.Subject
  if ($subject -notmatch '(^|, )CN=Microsoft Corporation(,|$)') {
    throw "fetch-vcrt: $name is not signed by Microsoft Corporation (signer: $subject)"
  }
}

$crt = Find-CrtDir
Write-Log "using $crt"

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("glint-vcrt-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
  foreach ($name in $Names) {
    $src = Join-Path $crt $name
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) { throw "fetch-vcrt: $name not found in $crt" }
    $copy = Join-Path $work $name # lowercase, whatever the source casing
    Copy-Item -LiteralPath $src -Destination $copy
    Assert-MicrosoftSigned $copy
  }

  New-Item -ItemType Directory -Force -Path $Dest | Out-Null
  # Remove only what this script (or fetch-vcrt.sh) put there, then install.
  foreach ($name in ($Names + $Marker)) {
    Remove-Item -LiteralPath (Join-Path $Dest $name) -Force -ErrorAction SilentlyContinue
  }
  foreach ($name in $Names) {
    Move-Item -LiteralPath (Join-Path $work $name) -Destination (Join-Path $Dest $name)
    $item = Get-Item -LiteralPath (Join-Path $Dest $name)
    Write-Log ("  {0}  {1} bytes  {2}" -f $item.Name, $item.Length, $item.VersionInfo.FileVersion)
  }
}
finally {
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Log "VC++ runtime DLLs written to $Dest"
