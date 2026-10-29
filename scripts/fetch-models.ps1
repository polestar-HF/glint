<#
.SYNOPSIS
  Fetch the PP-OCRv6 small detection/recognition ONNX models used by Glint.

.DESCRIPTION
  Source: the PyPI wheel rapidocr==3.10.0 (reachable from GitHub runners and
  from restricted dev containers). The wheel is downloaded with
  `pip download --require-hashes`, then only the model files are extracted and
  every file is verified against the SHA-256 pinned below.

  Idempotent: when all files are already present with the right checksum
  nothing is downloaded. Files are written only after verification; all scratch
  work happens in a temp dir, so no stray files can end up in the directory
  that Tauri bundles.

  Works in Windows PowerShell 5.1 and PowerShell 7+. Needs Python 3 with pip.
  Keep the pinned values in sync with scripts/fetch-models.sh (CI checks it).

.PARAMETER Dest
  Output directory. Default: src-tauri/resources/models/v6-small

.PARAMETER Force
  Re-download even if the files are up to date.

.PARAMETER Check
  Only verify; exit 1 if anything is missing or wrong (no network).

.PARAMETER WithDict
  Also write dict.txt, the recognition model's character list, extracted from
  the "character" metadata of rec.onnx.
#>
[CmdletBinding()]
param(
  [string]$Dest,
  [switch]$Force,
  [switch]$Check,
  [switch]$WithDict
)

$ErrorActionPreference = 'Stop'

# --- Pinned inputs -----------------------------------------------------------
$RapidocrVersion = '3.10.0'
$RapidocrWheelSha256 = '2fc34e26cd0f48514804a94f8832283039c7334076bff60768b1fed9ad068050'

# Extracted from the wheel. To ship another model file from the same wheel, add
# an entry here (and the matching line in fetch-models.sh).
$ModelFiles = @(
  @{ Name = 'det.onnx'; Member = 'rapidocr/models/PP-OCRv6_det_small.onnx'; Sha256 = '090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f' },
  @{ Name = 'rec.onnx'; Member = 'rapidocr/models/PP-OCRv6_rec_small.onnx'; Sha256 = '6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884' }
)

# Derived from rec.onnx (the wheel ships no separate dictionary file); only
# produced with -WithDict. Deterministic, so it is pinned like the rest.
$DictName = 'dict.txt'
$DictSource = 'rec.onnx'
$DictMetadataKey = 'character'
$DictSha256 = 'f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e'
# -----------------------------------------------------------------------------

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Dest) {
  $Dest = Join-Path $RepoRoot 'src-tauri/resources/models/v6-small'
}
# Absolute path: .NET file APIs resolve relative paths against the process
# directory, which is not necessarily the PowerShell location.
$Dest = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Dest)

function Write-Log([string]$Message) { Write-Host "fetch-models: $Message" }

function Get-Sha256([string]$Path) {
  return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

# True when $Path exists and has SHA-256 $Sha256.
function Test-FileOk([string]$Path, [string]$Sha256) {
  return ((Test-Path -LiteralPath $Path -PathType Leaf) -and ((Get-Sha256 $Path) -eq $Sha256))
}

function Test-AllOk {
  foreach ($m in $ModelFiles) {
    if (-not (Test-FileOk (Join-Path $Dest $m.Name) $m.Sha256)) { return $false }
  }
  if ($WithDict -and -not (Test-FileOk (Join-Path $Dest $DictName) $DictSha256)) { return $false }
  return $true
}

function Write-Report {
  Get-ChildItem -LiteralPath $Dest -File | ForEach-Object {
    Write-Log ("  {0}  {1} bytes" -f $_.Name, $_.Length)
  }
}

if ((-not $Force) -and (Test-AllOk)) {
  Write-Log "models up to date in $Dest"
  Write-Report
  exit 0
}
if ($Check) {
  throw "fetch-models: models missing or checksum mismatch in $Dest"
}

# Find a Python 3 interpreter: python, python3, or the py launcher.
$PythonCmd = $null
foreach ($candidate in @('python', 'python3')) {
  if (Get-Command $candidate -ErrorAction SilentlyContinue) { $PythonCmd = @($candidate); break }
}
if (-not $PythonCmd -and (Get-Command py -ErrorAction SilentlyContinue)) { $PythonCmd = @('py', '-3') }
if (-not $PythonCmd) { throw 'fetch-models: python (3.8+ with pip) is required to download the wheel' }
function Invoke-Python {
  $exe = $PythonCmd[0]
  $pre = @($PythonCmd | Select-Object -Skip 1)
  & $exe @pre @args
  if ($LASTEXITCODE -ne 0) { throw "fetch-models: python failed (exit code $LASTEXITCODE): $args" }
}

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("glint-models-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Work | Out-Null

try {
  Write-Log "downloading rapidocr==$RapidocrVersion from PyPI"
  $req = Join-Path $Work 'requirements.txt'
  $wheelDir = Join-Path $Work 'wheel'
  # ASCII, no BOM: pip chokes on a UTF-8 BOM in requirements files under 5.1.
  Set-Content -LiteralPath $req -Encoding ascii -Value "rapidocr==$RapidocrVersion --hash=sha256:$RapidocrWheelSha256"
  $pipArgs = @(
    '-m', 'pip', 'download', '--disable-pip-version-check', '--quiet', '--no-deps',
    '--only-binary=:all:', '--require-hashes', '-r', $req, '-d', $wheelDir
  )
  Invoke-Python @pipArgs

  $wheel = Get-ChildItem -LiteralPath $wheelDir -Filter 'rapidocr-*.whl' | Select-Object -First 1
  if (-not $wheel) { throw 'fetch-models: pip did not produce a rapidocr wheel' }
  if (-not (Test-FileOk $wheel.FullName $RapidocrWheelSha256)) {
    throw "fetch-models: wheel checksum mismatch: $($wheel.FullName)"
  }

  # Read only the named members; nothing else from the archive is extracted.
  $out = Join-Path $Work 'out'
  New-Item -ItemType Directory -Path $out | Out-Null
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [System.IO.Compression.ZipFile]::OpenRead($wheel.FullName)
  try {
    foreach ($m in $ModelFiles) {
      $entry = $zip.GetEntry($m.Member)
      if (-not $entry) { throw "fetch-models: $($m.Member) not found in $($wheel.Name)" }
      [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $out $m.Name), $true)
    }
  }
  finally { $zip.Dispose() }

  foreach ($m in $ModelFiles) {
    $p = Join-Path $out $m.Name
    if (-not (Test-FileOk $p $m.Sha256)) {
      throw "fetch-models: checksum mismatch for $($m.Name) (expected $($m.Sha256), got $(Get-Sha256 $p))"
    }
  }

  if ($WithDict) {
    $dictOut = Join-Path $out $DictName
    $metaArgs = @('-I', (Join-Path $PSScriptRoot 'onnx_meta.py'), (Join-Path $out $DictSource), $DictMetadataKey, $dictOut)
    Invoke-Python @metaArgs
    if (-not (Test-FileOk $dictOut $DictSha256)) { throw "fetch-models: checksum mismatch for $DictName" }
  }

  New-Item -ItemType Directory -Force -Path $Dest | Out-Null
  Get-ChildItem -LiteralPath $out -File | ForEach-Object {
    Move-Item -LiteralPath $_.FullName -Destination (Join-Path $Dest $_.Name) -Force
  }
}
finally {
  Remove-Item -LiteralPath $Work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Log "models written to $Dest"
Write-Report
