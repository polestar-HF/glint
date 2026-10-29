#!/usr/bin/env bash
# Fetch the CPU ONNX Runtime DLL (Windows x64) that glint-ocr loads at run time.
#
#   scripts/fetch-runtime.sh [--dest FILE] [--force] [--check]
#
# Source: the PyPI wheel onnxruntime==1.31.1 for Windows x64, fetched with
# `pip download --require-hashes` (pip can download Windows wheels on any OS).
# Only onnxruntime/capi/onnxruntime.dll is extracted, and both the wheel and the
# DLL are verified against the SHA-256 pinned below.
#
# Idempotent: when the DLL is already present with the right checksum nothing is
# downloaded. The file is written only after verification, and all scratch work
# happens in a temp dir.
#
#   --dest FILE  output file (default: src-tauri/resources/onnxruntime.dll)
#   --force      re-download even if the file is up to date
#   --check      only verify; exit 1 if missing or wrong (no network)
#
# Keep the pinned values in sync with scripts/fetch-runtime.ps1 (CI checks it).
# A tauri.conf.json resource entry maps the file next to glint.exe.
set -euo pipefail

# --- Pinned inputs -----------------------------------------------------------
ORT_VERSION="1.31.1"
# pip platform selection (any host OS): CPython 3.13, Windows x64.
ORT_PIP_PLATFORM="win_amd64"
ORT_PIP_PYTHON_VERSION="3.13"
ORT_PIP_ABI="cp313"
ORT_WHEEL_SHA256="0558f29d5f6ac09957390c9c5495931485ee8ae7f0ce781ccee86a0ae5c2348b"
ORT_DLL_MEMBER="onnxruntime/capi/onnxruntime.dll"
ORT_DLL_SHA256="68d0d5f502be4509e2a16f932627a95137d748b2fae519027f80125c0b1f550b"
# -----------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST="$REPO_ROOT/src-tauri/resources/onnxruntime.dll"
FORCE=0
CHECK=0

die() { echo "fetch-runtime: error: $*" >&2; exit 1; }
log() { echo "fetch-runtime: $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dest) [ $# -ge 2 ] || die "--dest needs a file path"; DEST="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --check) CHECK=1; shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# True when "$1" exists and has SHA-256 "$2".
file_ok() { [ -f "$1" ] && [ "$(sha256_of "$1")" = "$2" ]; }

report() { log "  $(basename "$DEST")  $(wc -c <"$DEST" | tr -d ' ') bytes"; }

if [ "$FORCE" = 0 ] && file_ok "$DEST" "$ORT_DLL_SHA256"; then
  log "onnxruntime.dll up to date: $DEST"
  report
  exit 0
fi
[ "$CHECK" = 0 ] || die "onnxruntime.dll missing or checksum mismatch: $DEST"

PYTHON=""
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1; then PYTHON="$candidate"; break; fi
done
[ -n "$PYTHON" ] || die "python (3.8+ with pip) is required to download the wheel"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

log "downloading onnxruntime==$ORT_VERSION ($ORT_PIP_PLATFORM wheel) from PyPI"
printf 'onnxruntime==%s --hash=sha256:%s\n' "$ORT_VERSION" "$ORT_WHEEL_SHA256" >"$WORK/requirements.txt"
"$PYTHON" -m pip download --disable-pip-version-check --quiet --no-deps --only-binary=:all: \
  --platform "$ORT_PIP_PLATFORM" --python-version "$ORT_PIP_PYTHON_VERSION" \
  --implementation cp --abi "$ORT_PIP_ABI" \
  --require-hashes -r "$WORK/requirements.txt" -d "$WORK/wheel"

WHEEL=""
for f in "$WORK"/wheel/onnxruntime-*.whl; do
  if [ -f "$f" ]; then WHEEL="$f"; break; fi
done
[ -n "$WHEEL" ] || die "pip did not produce an onnxruntime wheel"
file_ok "$WHEEL" "$ORT_WHEEL_SHA256" || die "wheel checksum mismatch: $WHEEL"

# Read only the named member; nothing else from the archive is extracted.
"$PYTHON" -I - "$WHEEL" "$ORT_DLL_MEMBER" "$WORK/onnxruntime.dll" <<'PY'
import shutil, sys, zipfile
wheel, member, out = sys.argv[1:]
with zipfile.ZipFile(wheel) as z, z.open(member) as src, open(out, "wb") as dst:
    shutil.copyfileobj(src, dst)
PY
file_ok "$WORK/onnxruntime.dll" "$ORT_DLL_SHA256" \
  || die "checksum mismatch for onnxruntime.dll (expected $ORT_DLL_SHA256, got $(sha256_of "$WORK/onnxruntime.dll"))"

mkdir -p "$(dirname "$DEST")"
mv -f "$WORK/onnxruntime.dll" "$DEST"

log "onnxruntime.dll written to $DEST"
report
