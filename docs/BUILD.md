# Building Glint

Glint ships as a Windows x64 NSIS installer and a portable zip. Both are built by
GitHub Actions; this page covers local development, the model download, releases and
expected sizes.

## Local Windows setup

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.97.0 (what CI pins) | `rustup default 1.97.0`, target `x86_64-pc-windows-msvc` |
| MSVC | VS 2022 Build Tools | workload "Desktop development with C++" (MSVC v143 + Windows SDK) |
| Node.js | 22 LTS | npm comes with it |
| WebView2 Runtime | any current | preinstalled on Windows 11 and up-to-date Windows 10; otherwise install the Evergreen runtime from Microsoft |
| Python | 3.8+ with pip | only used by the model download script |
| Tauri CLI | 2.12.x | `cargo install tauri-cli --version "^2.12" --locked`, or prefix commands with `npx @tauri-apps/cli@2.12.4` instead of `cargo tauri` |

```powershell
git clone <repo> glint; cd glint
powershell -ExecutionPolicy Bypass -File scripts/fetch-models.ps1   # models, once
npm ci --prefix ui             # UI dependencies
cargo tauri dev                # Vite dev server + app, from the repo root
```

`cargo tauri dev` runs `beforeDevCommand` (`npm run dev` in `ui/`, Vite on :5173).
`cargo tauri build --bundles nsis` produces a local installer in `target/release/bundle/nsis/`
(the first run downloads NSIS).

Order matters for any cargo command on the `glint` crate (`cargo check`, `clippy`, `test`):

1. Models must exist: `tauri-build` copies `bundle.resources` and fails on a missing glob.
2. `ui/dist` must exist: the crate embeds it at compile time. Run `npm run build` in `ui/` first.

## Models

`scripts/fetch-models.ps1` (Windows) and `scripts/fetch-models.sh` (Linux/macOS) download the
PP-OCRv6 small detection and recognition models into `src-tauri/resources/models/v6-small/`
(`det.onnx`, `rec.onnx`). The folder is git-ignored; never commit models.

- **Source:** the PyPI wheel `rapidocr==3.10.0`, fetched with `pip download --require-hashes`;
  only the two `.onnx` members are extracted. PyPI is reachable from GitHub runners and from the
  restricted dev containers; HuggingFace and `cdn.pyke.io` are not always.
- **Verified:** the wheel and each model are checked against pinned SHA-256 values.
- **Idempotent:** if the files are present and correct, nothing is downloaded. `-Force` / `--force`
  re-downloads, `-Check` / `--check` only verifies (exit 1 on mismatch, no network).
- **`dict.txt`:** not shipped in the wheel. The recognition model embeds its character list as
  ONNX metadata `character`, so `ort` can read it directly. If a file is preferred, pass
  `-WithDict` / `--with-dict` (extracted by `scripts/onnx_meta.py`) and set `MODELS_WITH_DICT: "true"`
  in `ci.yml` and `windows-build.yml`.

| File | SHA-256 | Size |
|---|---|---|
| wheel `rapidocr-3.10.0-py3-none-any.whl` | `2fc34e26cd0f48514804a94f8832283039c7334076bff60768b1fed9ad068050` | 27.3 MB |
| `det.onnx` (`PP-OCRv6_det_small.onnx`) | `090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f` | 9,929,594 B |
| `rec.onnx` (`PP-OCRv6_rec_small.onnx`) | `6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884` | 21,234,383 B |
| `dict.txt` (from `rec.onnx`, optional) | `f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e` | 74,946 B |

To change models: edit the version, member paths and hashes in **both** scripts (`ci.yml` fails if
they disagree). CI caches the folder under a key derived from the two scripts, so the cache refreshes
automatically.

## CI

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to any branch, PR, manual | `rust-linux`: `cargo fmt --check`, clippy and tests for `glint-ocr` (`ort-dynamic`), `glint-translate`, `glint-overlay`. `windows`: calls `windows-build.yml`. |
| `windows-build.yml` | reusable | Clippy (`--workspace`, `-D warnings`) and tests on Windows, `tauri build --bundles nsis`, portable zip, artifacts `glint-installer` and `glint-portable`, size table in the job summary. |
| `release.yml` | tag `v*` | Checks tag = `Cargo.toml` = `tauri.conf.json` version, runs `windows-build.yml`, creates a **draft** GitHub Release with installer, zip and `SHA256SUMS.txt`. |

Cargo flags for the Linux job (`OCR_FLAGS`) and the Windows scopes (`CLIPPY_SCOPE`, `TEST_SCOPE`)
are `env:` entries at the top of the workflows. Tool versions (Rust, Node, Tauri CLI) are pinned
there too. Cargo caches are written by the default branch only; PRs and tags restore them.

## Cutting a release

1. Bump `version` in `Cargo.toml` (`[workspace.package]`) and `src-tauri/tauri.conf.json`; merge to `main`
   with green CI.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`. Use `v0.1.0-rc.1` for a pre-release
   (marked as such automatically).
3. Wait for `Release`. A version/tag mismatch fails the first job.
4. Open the draft release, check the sizes in the build job summary, test the installer, then publish.
   To redo a bad build, delete the draft and the tag and push the tag again.

Code signing is not enabled: Windows SmartScreen will warn on the installer. A commented placeholder in
`windows-build.yml` shows where to add a certificate from repository secrets.

## Expected sizes

Size is a product KPI (installer under 45 MB per `ARCHITECTURE.md`; CI measures MiB). The job summary lists every
artifact and warns, without failing, when the installer is over budget.

| Part | Size |
|---|---|
| Models, unpacked | 29.7 MiB (det 9.5, rec 20.3) |
| Models, LZMA/deflate (measured) | about 24.8 MiB / 25.3 MiB; ONNX weights barely compress |
| `glint.exe` | 4-8 MiB, plus 12-20 MiB if ONNX Runtime is linked statically (estimates) |
| NSIS installer | about 40-50 MiB (estimate), WebView2 is downloaded, not embedded |
| Portable zip | about the same as the installer |

## Troubleshooting

- `frontendDist ... doesn't exist` or a missing-resource error: see the order of steps above.
- ONNX Runtime link errors after a cache restore: the `ort.pyke.io` download folder must be cached
  together with `target/` (it is, via `cache-directories` in `windows-build.yml`).
- `cargo test` on Windows dying with `0xc0000139` (STATUS_ENTRYPOINT_NOT_FOUND) in the `glint` crate:
  the test binary lacks the Common Controls manifest. Exclude it with
  `TEST_SCOPE: "--workspace --exclude glint"` or embed the manifest for tests.
- Linux container: `cargo check --target x86_64-pc-windows-msvc -p <crate>` type-checks the Windows
  code; `ort-dynamic` tests need `ORT_DYLIB_PATH` pointing at a `libonnxruntime.so` (see the
  "Fetch ONNX Runtime" step in `ci.yml`).
