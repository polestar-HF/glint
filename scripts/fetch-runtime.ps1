<#
.SYNOPSIS
  Fetch the CPU ONNX Runtime DLL (Windows x64) that glint-ocr loads at run time.

.DESCRIPTION
  Source: the PyPI wheel onnxruntime==1.31.1 for Windows x64, fetched with
  `pip download --require-hashes`. Only onnxruntime/capi/onnxruntime.dll is
  extracted, and both the wheel and the DLL are verified against the SHA-256
  pinned below.

  Idempotent: when the DLL is already present with the right checksum nothing
  is downloaded. The file is written only after verification, and all scratch
  work happens in a temp dir.

  Works in Windows PowerShell 5.1 and PowerShell 7+. Needs Python 3 with pip.
  Keep the pinned values in sync with scripts/fetch-runtime.sh (CI checks it).
  A tauri.conf.json resource entry maps the file next to glint.exe.

.PARAMETER Dest
  Output file. Default: src-tauri/resources/onnxruntime.dll

.PARAMETER Force
  Re-download even if the file is up to date.

.PARAMETER Check
  Only verify; exit 1 if missing or wrong (no network).
#>
[CmdletBinding()]
param(
  [string]$Dest,
  [switch]$Force,
  [switch]$Check
)

$ErrorActionPreference = 'Stop'

# --- Pinned inputs -----------------------------------------------------------
$OrtVersion = '1.31.1'
# pip platform selection (any host OS): CPython 3.13, Windows x64.
$OrtPipPlatform = 'win_amd64'
$OrtPipPythonVersion = '3.13'
$OrtPipAbi = 'cp313'
$OrtWheelSha256 = '0558f29d5f6ac09957390c9c5495931485ee8ae7f0ce781ccee86a0ae5c2348b'
$OrtDllMember = 'onnxruntime/capi/onnxruntime.dll'
$OrtDllSha256 = '68d0d5f502be4509e2a16f932627a95137d748b2fae519027f80125c0b1f550b'
# -----------------------------------------------------------------------------

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Dest) {
  $Dest = Join-Path $RepoRoot 'src-tauri/resources/onnxruntime.dll'
}
# Absolute path: .NET file APIs resolve relative paths against the process
# directory, which is not necessarily the PowerShell location.
$Dest = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Dest)

function Write-Log([string]$Message) { Write-Host "fetch-runtime: $Message" }

function Get-Sha256([string]$Path) {
  return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

# True when $Path exists and has SHA-256 $Sha256.
function Test-FileOk([string]$Path, [string]$Sha256) {
  return ((Test-Path -LiteralPath $Path -PathType Leaf) -and ((Get-Sha256 $Path) -eq $Sha256))
}

function Write-Report {
  $item = Get-Item -LiteralPath $Dest
  Write-Log ("  {0}  {1} bytes" -f $item.Name, $item.Length)
}

if ((-not $Force) -and (Test-FileOk $Dest $OrtDllSha256)) {
  Write-Log "onnxruntime.dll up to date: $Dest"
  Write-Report
  exit 0
}
if ($Check) {
  throw "fetch-runtime: onnxruntime.dll missing or checksum mismatch: $Dest"
}

# Find a Python 3 interpreter: python, python3, or the py launcher.
$PythonCmd = $null
foreach ($candidate in @('python', 'python3')) {
  if (Get-Command $candidate -ErrorAction SilentlyContinue) { $PythonCmd = @($candidate); break }
}
if (-not $PythonCmd -and (Get-Command py -ErrorAction SilentlyContinue)) { $PythonCmd = @('py', '-3') }
if (-not $PythonCmd) { throw 'fetch-runtime: python (3.8+ with pip) is required to download the wheel' }
function Invoke-Python {
  $exe = $PythonCmd[0]
  $pre = @($PythonCmd | Select-Object -Skip 1)
  & $exe @pre @args
  if ($LASTEXITCODE -ne 0) { throw "fetch-runtime: python failed (exit code $LASTEXITCODE): $args" }
}

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("glint-runtime-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Work | Out-Null

try {
  Write-Log "downloading onnxruntime==$OrtVersion ($OrtPipPlatform wheel) from PyPI"
  $req = Join-Path $Work 'requirements.txt'
  $wheelDir = Join-Path $Work 'wheel'
  # ASCII, no BOM: pip chokes on a UTF-8 BOM in requirements files under 5.1.
  Set-Content -LiteralPath $req -Encoding ascii -Value "onnxruntime==$OrtVersion --hash=sha256:$OrtWheelSha256"
  $pipArgs = @(
    '-m', 'pip', 'download', '--disable-pip-version-check', '--quiet', '--no-deps',
    '--only-binary=:all:', '--platform', $OrtPipPlatform, '--python-version', $OrtPipPythonVersion,
    '--implementation', 'cp', '--abi', $OrtPipAbi,
    '--require-hashes', '-r', $req, '-d', $wheelDir
  )
  Invoke-Python @pipArgs

  $wheel = Get-ChildItem -LiteralPath $wheelDir -Filter 'onnxruntime-*.whl' | Select-Object -First 1
  if (-not $wheel) { throw 'fetch-runtime: pip did not produce an onnxruntime wheel' }
  if (-not (Test-FileOk $wheel.FullName $OrtWheelSha256)) {
    throw "fetch-runtime: wheel checksum mismatch: $($wheel.FullName)"
  }

  # Read only the named member; nothing else from the archive is extracted.
  $dll = Join-Path $Work 'onnxruntime.dll'
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [System.IO.Compression.ZipFile]::OpenRead($wheel.FullName)
  try {
    $entry = $zip.GetEntry($OrtDllMember)
    if (-not $entry) { throw "fetch-runtime: $OrtDllMember not found in $($wheel.Name)" }
    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dll, $true)
  }
  finally { $zip.Dispose() }

  if (-not (Test-FileOk $dll $OrtDllSha256)) {
    throw "fetch-runtime: checksum mismatch for onnxruntime.dll (expected $OrtDllSha256, got $(Get-Sha256 $dll))"
  }

  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Dest) | Out-Null
  Move-Item -LiteralPath $dll -Destination $Dest -Force
}
finally {
  Remove-Item -LiteralPath $Work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Log "onnxruntime.dll written to $Dest"
Write-Report
