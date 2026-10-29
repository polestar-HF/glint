#!/usr/bin/env bash
# Make the four VC++ runtime files exist locally so that Tauri's build script finds
# its mapped resources. For type-checking on Linux only: NOT SHIPPABLE.
#
#   scripts/fetch-vcrt.sh [--dest DIR] [--force]
#
# The real DLLs (msvcp140.dll, msvcp140_1.dll, vcruntime140.dll, vcruntime140_1.dll)
# come from the Visual Studio redistributable folder; scripts/fetch-vcrt.ps1 copies
# and signature-checks them on Windows, and CI does exactly that before building.
# There is no source we can verify from Linux, so this script writes empty
# placeholders instead, plus a marker file, and warns loudly. windows-build.yml
# fails if any of these files is smaller than 10 KB, so a placeholder can never end
# up in an installer or zip built by CI.
#
# Idempotent: files that already exist (real or placeholder) are left alone.
#
#   --dest DIR   output directory (default: src-tauri/resources/vcrt)
#   --force      replace existing files with fresh placeholders
set -euo pipefail

NAMES=(msvcp140.dll msvcp140_1.dll vcruntime140.dll vcruntime140_1.dll)
MARKER="PLACEHOLDER-NOT-SHIPPABLE.txt"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST="$REPO_ROOT/src-tauri/resources/vcrt"
FORCE=0

die() { echo "fetch-vcrt: error: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dest) [ $# -ge 2 ] || die "--dest needs a directory"; DEST="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

mkdir -p "$DEST"
created=0
for name in "${NAMES[@]}"; do
  if [ "$FORCE" = 1 ] || [ ! -e "$DEST/$name" ]; then
    : >"$DEST/$name"
    created=$((created + 1))
  fi
done

placeholders=0
for name in "${NAMES[@]}"; do
  if [ "$(wc -c <"$DEST/$name" | tr -d ' ')" -lt 10240 ]; then placeholders=$((placeholders + 1)); fi
done

if [ "$placeholders" -eq 0 ]; then
  rm -f "$DEST/$MARKER"
  echo "fetch-vcrt: all four files in $DEST are >= 10 KB (real DLLs); left untouched"
  exit 0
fi

cat >"$DEST/$MARKER" <<EOF
The *.dll files in this folder are EMPTY PLACEHOLDERS written by scripts/fetch-vcrt.sh
so that the Tauri build script (and cargo check) find their mapped resources.
They are NOT the Visual C++ runtime and must never be shipped.
Real builds copy the signed DLLs from Visual Studio: scripts/fetch-vcrt.ps1 (CI does this).
EOF

echo "fetch-vcrt: wrote $created empty placeholder(s) to $DEST ($placeholders in total)" >&2
echo "fetch-vcrt: WARNING: these files are EMPTY PLACEHOLDERS, NOT SHIPPABLE." >&2
echo "fetch-vcrt: WARNING: they only satisfy 'cargo check --target x86_64-pc-windows-msvc'." >&2
echo "fetch-vcrt: WARNING: release builds use the signed DLLs copied by scripts/fetch-vcrt.ps1 in CI." >&2
