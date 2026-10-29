#!/usr/bin/env bash
# Fetch the PP-OCRv6 small detection/recognition ONNX models used by Glint.
#
#   scripts/fetch-models.sh [--dest DIR] [--force] [--check] [--with-dict]
#
# Source: the PyPI wheel rapidocr==3.10.0 (reachable from GitHub runners and
# from the restricted dev containers). The wheel is downloaded with
# `pip download --require-hashes`, then only the model files are extracted
# and every file is verified against the SHA-256 pinned below.
#
# Idempotent: when all files are already present with the right checksum
# nothing is downloaded. Files are written only after verification; all
# scratch work happens in a temp dir, so no stray files can end up in the
# directory that Tauri bundles.
#
#   --dest DIR   output directory (default: src-tauri/resources/models/v6-small)
#   --force      re-download even if the files are up to date
#   --check      only verify; exit 1 if anything is missing or wrong (no network)
#   --with-dict  also write dict.txt, the recognition model's character list,
#                extracted from the "character" metadata of rec.onnx
#
# Keep the pinned values in sync with scripts/fetch-models.ps1 (CI checks it).
set -euo pipefail

# --- Pinned inputs -----------------------------------------------------------
RAPIDOCR_VERSION="3.10.0"
RAPIDOCR_WHEEL_SHA256="2fc34e26cd0f48514804a94f8832283039c7334076bff60768b1fed9ad068050"

# Extracted from the wheel: "<output name>|<path inside the wheel>|<sha256>".
# To ship another model file from the same wheel, add a line here (and the
# matching entry in fetch-models.ps1).
MODEL_FILES=(
  "det.onnx|rapidocr/models/PP-OCRv6_det_small.onnx|090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f"
  "rec.onnx|rapidocr/models/PP-OCRv6_rec_small.onnx|6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884"
)

# Derived from rec.onnx (the wheel ships no separate dictionary file); only
# produced with --with-dict. Deterministic, so it is pinned like the rest.
DICT_NAME="dict.txt"
DICT_SOURCE="rec.onnx"
DICT_METADATA_KEY="character"
DICT_SHA256="f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e"
# -----------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST="$REPO_ROOT/src-tauri/resources/models/v6-small"
FORCE=0
CHECK=0
WITH_DICT=0

die() { echo "fetch-models: error: $*" >&2; exit 1; }
log() { echo "fetch-models: $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dest) [ $# -ge 2 ] || die "--dest needs a directory"; DEST="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --check) CHECK=1; shift ;;
    --with-dict) WITH_DICT=1; shift ;;
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

all_ok() {
  local entry name sha
  for entry in "${MODEL_FILES[@]}"; do
    IFS='|' read -r name _ sha <<<"$entry"
    file_ok "$DEST/$name" "$sha" || return 1
  done
  if [ "$WITH_DICT" = 1 ]; then
    file_ok "$DEST/$DICT_NAME" "$DICT_SHA256" || return 1
  fi
  return 0
}

report() {
  local f
  for f in "$DEST"/*; do
    [ -f "$f" ] && log "  $(basename "$f")  $(wc -c <"$f" | tr -d ' ') bytes"
  done
  return 0
}

if [ "$FORCE" = 0 ] && all_ok; then
  log "models up to date in $DEST"
  report
  exit 0
fi
[ "$CHECK" = 0 ] || die "models missing or checksum mismatch in $DEST"

PYTHON=""
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1; then PYTHON="$candidate"; break; fi
done
[ -n "$PYTHON" ] || die "python (3.8+ with pip) is required to download the wheel"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

log "downloading rapidocr==$RAPIDOCR_VERSION from PyPI"
printf 'rapidocr==%s --hash=sha256:%s\n' "$RAPIDOCR_VERSION" "$RAPIDOCR_WHEEL_SHA256" >"$WORK/requirements.txt"
"$PYTHON" -m pip download --disable-pip-version-check --quiet --no-deps --only-binary=:all: \
  --require-hashes -r "$WORK/requirements.txt" -d "$WORK/wheel"

WHEEL=""
for f in "$WORK"/wheel/rapidocr-*.whl; do
  if [ -f "$f" ]; then WHEEL="$f"; break; fi
done
[ -n "$WHEEL" ] || die "pip did not produce a rapidocr wheel"
file_ok "$WHEEL" "$RAPIDOCR_WHEEL_SHA256" || die "wheel checksum mismatch: $WHEEL"

# Read only the named members; nothing else from the archive is extracted.
mkdir -p "$WORK/out"
specs=()
for entry in "${MODEL_FILES[@]}"; do
  IFS='|' read -r name member _ <<<"$entry"
  specs+=("$member=$WORK/out/$name")
done
"$PYTHON" -I - "$WHEEL" "${specs[@]}" <<'PY'
import shutil, sys, zipfile
wheel, specs = sys.argv[1], sys.argv[2:]
with zipfile.ZipFile(wheel) as z:
    for spec in specs:
        member, out = spec.split("=", 1)
        with z.open(member) as src, open(out, "wb") as dst:
            shutil.copyfileobj(src, dst)
PY

for entry in "${MODEL_FILES[@]}"; do
  IFS='|' read -r name _ sha <<<"$entry"
  file_ok "$WORK/out/$name" "$sha" || die "checksum mismatch for $name (expected $sha, got $(sha256_of "$WORK/out/$name"))"
done

if [ "$WITH_DICT" = 1 ]; then
  "$PYTHON" -I "$SCRIPT_DIR/onnx_meta.py" "$WORK/out/$DICT_SOURCE" "$DICT_METADATA_KEY" "$WORK/out/$DICT_NAME" \
    || die "could not extract $DICT_NAME from $DICT_SOURCE"
  file_ok "$WORK/out/$DICT_NAME" "$DICT_SHA256" || die "checksum mismatch for $DICT_NAME"
fi

mkdir -p "$DEST"
for f in "$WORK"/out/*; do
  mv -f "$f" "$DEST/$(basename "$f")"
done

log "models written to $DEST"
report
