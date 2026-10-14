# CI and installer builds

One GitHub Actions workflow, [`.github/workflows/ci.yml`](../.github/workflows/ci.yml), checks
every push to `main` and every pull request and, if everything passes, builds the Windows x64
NSIS installer from that same checkout.

**Status: one green run; limits not yet reviewed.** The workflow first ran on 2026-10-13
against commit `e634751` (the scaffold and library layer) and passed on its first attempt:
[run 38430911426](https://github.com/test-dan-run/skribe/actions/runs/38430911426). The limits
below are decision D5 in [IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md) and still await
the owner's review. The work added since that run (meetings, recording, and the Windows audio
module, which only this job compiles) has not yet been through CI.

## What the job does

There is one job, `check-and-package`, and its steps run strictly in order. Any failing step
stops the job, so the installer cannot be built from a revision that failed a check.

| # | Step | Command |
|---|------|---------|
| 1 | Check out; install Node from `.node-version` and Rust from `rust-toolchain.toml` | |
| 2 | Install JavaScript dependencies from the lockfile | `npm ci` |
| 3 | Lint, type-check, frontend tests | `npm run lint`, `npm run typecheck`, `npm test` |
| 4 | Build the frontend bundle (the app crate embeds it, so clippy needs it) | `npm run build` |
| 5 | Rust formatting, clippy with warnings denied, tests | `npm run rust:fmt`, `npm run rust:clippy`, `npm run rust:test` |
| 6 | Build the installer: the only release compile | `npx tauri build --ci -- --locked` |
| 7 | Checksums, manifests, budget checks | `node scripts/ci/package-manifests.mjs` |
| 8 | Upload the artifact | |

Things worth knowing:

- `cargo test` builds only the library crates. The Tauri app crate has no tests of its own, so
  no test binary links the webview. Everything that needs testing lives in `crates/`.
- The Rust tests fail if `src/ipc/generated` does not match the Rust command contract. Run
  `npm run ipc:generate` and commit the result to fix that.
- SQLite is compiled from the C source bundled with `rusqlite`, as part of the same job. CI
  builds no inference runtime and downloads no model.
- The Windows audio module (`crates/skribe-audio/src/wasapi.rs`) is compiled only on Windows,
  so this job is its only full build. It can be type-checked in WSL with
  `cargo check -p skribe-audio --target x86_64-pc-windows-msvc` once that target is installed
  with `rustup target add`. No CI step can exercise it: hosted runners have no audio devices.
- The installer build compiles only the app and what it depends on. The synthetic capture
  backend is a test fixture behind a feature that build does not enable, and a release build
  that enabled it would fail to compile.
- Actions are pinned to commit SHAs. The job's token can only read the repository.

## Resource limits

| Limit | Value | Why |
|-------|-------|-----|
| Runner | `windows-2025`, standard GitHub-hosted | Pinned image label rather than `windows-latest`, so an image change is a reviewed edit |
| Runner size | 4 vCPU, 16 GB RAM, 14 GB SSD | What GitHub documents for standard Windows runners on public repositories (checked 2026-10-13). A private repository gets 2 vCPU and 8 GB: lower `CARGO_BUILD_JOBS` to 2 if the repository's visibility changes |
| Job timeout | 60 minutes | The first run took 6 minutes 25 seconds with no build cache. A job that reaches the limit is treated as hung |
| Build parallelism | `CARGO_BUILD_JOBS=4` | One compiler process per vCPU, which leaves about 4 GB of RAM for each. Set explicitly so a different runner does not change it silently |
| Build disk | `target/` at most 10 GiB | Leaves room on the 14 GB disk. Checked in step 7 and recorded in the build manifest |
| Installer size | at most 30 MiB | Models and runtimes are never bundled. Checked in step 7 |
| Build cache | none | Every run compiles from the lockfiles. Slower, but nothing from an earlier run can reach the installer. Revisit with measured timings |
| Artifact retention | 30 days | CI artifacts are for review, not distribution |

### Measured on the first run

Run 38430911426, `windows-2025`, 2026-10-13, no build cache:

| Step | Time |
|------|------|
| Whole job | 6 min 25 s |
| Clippy (compiles every crate once, debug) | 1 min 46 s |
| Rust tests | 16 s |
| Build NSIS installer (the release compile) | 3 min 18 s |
| Everything else | under 1 min |

The uploaded artifact was 2.8 MB compressed, against the 30 MiB installer budget. The measured
`target/` size and the installer's exact size are in that run's `build-manifest.json`; they
were not read back into this document.

These figures predate the audio crate, the Windows API bindings it uses and the property-test
dependency, all of which add compile time. Re-measure after the next run.

## Artifact

One artifact per run, named `skribe-windows-x64-unsigned-<commit>`, containing:

- `Skribe_<version>_x64-setup.exe`: the NSIS installer.
- `SHA256SUMS.txt`: SHA-256 of the installer and of both manifests.
- `source-manifest.json`: commit, ref, SHA-256 of `package-lock.json` and `Cargo.lock`, the
  pinned toolchain files and the toolchain versions actually used.
- `build-manifest.json`: runner image and version, workflow run, the limits above, measured
  `target/` size, and the installer's size and SHA-256.

Step 7 refuses to package if the working tree differs from the checked-out commit.

## What a green run does not mean

A green run means the checks passed and an installer was produced. It does not make a release:
the installer is unsigned, has not been through Windows acceptance, and is not published. Those
are separate release steps outside this workflow.

The installer uses Tauri's default WebView2 bootstrapper. Windows 11 and current Windows 10
include WebView2; on a machine without it, the installer needs a connection to fetch it.
Bundling the offline WebView2 installer would avoid that at a cost of over 100 MB, which is why
it is not the default here.

## Running the same checks locally

In WSL, after `npm ci`:

```
npm run check        # lint, type-check, frontend tests
npm run build        # frontend bundle, needed before clippy
npm run check:rust   # rustfmt, clippy, Rust tests
```

WSL cannot produce the NSIS installer. `npx tauri build --no-bundle` compiles the release
binary for Linux, which is enough to check that the app starts.
