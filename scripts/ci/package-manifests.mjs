// Collects the NSIS installer with its checksums and source/build manifests into one folder
// for upload, and enforces the installer size and build disk budgets from docs/ci.md.
//
// Usage: node scripts/ci/package-manifests.mjs [bundle-dir] [output-dir]
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, join } from "node:path";

const MIB = 1024 * 1024;
const INSTALLER_BUDGET_BYTES = 30 * MIB;
const TARGET_DIR_BUDGET_BYTES = 10 * 1024 * MIB;

const bundleDir = process.argv[2] ?? "target/release/bundle/nsis";
const outputDir = process.argv[3] ?? "target/ci-artifacts";
const inCi = process.env.CI === "true";

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

/** Output of a command, or null when it cannot be run or fails. */
function capture(command, args) {
  // npm is a .cmd shim on Windows and needs a shell to start.
  const shell = process.platform === "win32" && command === "npm";
  const result = spawnSync(command, args, { encoding: "utf8", shell });
  return result.status === 0 ? result.stdout.trim() : null;
}

function directorySize(path) {
  let total = 0;
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) {
      total += directorySize(child);
    } else if (entry.isFile()) {
      total += statSync(child).size;
    }
  }
  return total;
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

// The installer, and only one: more than one means stale output from another revision.
const installers = readdirSync(bundleDir).filter((name) => name.endsWith("-setup.exe"));
if (installers.length !== 1) {
  fail(`expected exactly one *-setup.exe in ${bundleDir}, found ${String(installers.length)}`);
}
const installerName = installers[0];
const installerPath = join(bundleDir, installerName);
const installerBytes = statSync(installerPath).size;
if (installerBytes > INSTALLER_BUDGET_BYTES) {
  fail(
    `${installerName} is ${String(installerBytes)} bytes, over the ` +
      `${String(INSTALLER_BUDGET_BYTES)} byte installer budget. Models and runtimes must not ` +
      "be bundled; see docs/ci.md.",
  );
}

const targetDirBytes = directorySize("target");
if (targetDirBytes > TARGET_DIR_BUDGET_BYTES) {
  fail(
    `target/ is ${String(targetDirBytes)} bytes, over the ` +
      `${String(TARGET_DIR_BUDGET_BYTES)} byte disk budget in docs/ci.md.`,
  );
}

// The artifact must come from exactly the checked-out revision.
const commit = capture("git", ["rev-parse", "HEAD"]);
const status = capture("git", ["status", "--porcelain"]);
const treeClean = status === "";
if (inCi && (commit === null || !treeClean)) {
  fail(`the working tree does not match a commit, so the build is not traceable:\n${status ?? ""}`);
}

rmSync(outputDir, { recursive: true, force: true });
mkdirSync(outputDir, { recursive: true });
copyFileSync(installerPath, join(outputDir, installerName));

const env = process.env;
writeJson(join(outputDir, "source-manifest.json"), {
  commit,
  ref: env.GITHUB_REF ?? null,
  repository: env.GITHUB_REPOSITORY ?? null,
  tree_clean: treeClean,
  lockfiles_sha256: {
    "package-lock.json": sha256("package-lock.json"),
    "Cargo.lock": sha256("Cargo.lock"),
  },
  pinned: {
    "rust-toolchain.toml": readFileSync("rust-toolchain.toml", "utf8"),
    ".node-version": readFileSync(".node-version", "utf8").trim(),
  },
  toolchain: {
    rustc: capture("rustc", ["--version", "--verbose"]),
    cargo: capture("cargo", ["--version"]),
    node: process.version,
    npm: capture("npm", ["--version"]),
    tauri_cli: JSON.parse(readFileSync("node_modules/@tauri-apps/cli/package.json", "utf8")).version,
  },
});

writeJson(join(outputDir, "build-manifest.json"), {
  built_at: new Date().toISOString(),
  signed: false,
  release: false,
  runner: {
    os: env.RUNNER_OS ?? process.platform,
    arch: env.RUNNER_ARCH ?? process.arch,
    image: env.ImageOS ?? null,
    image_version: env.ImageVersion ?? null,
    name: env.RUNNER_NAME ?? null,
  },
  workflow: {
    name: env.GITHUB_WORKFLOW ?? null,
    run_id: env.GITHUB_RUN_ID ?? null,
    run_attempt: env.GITHUB_RUN_ATTEMPT ?? null,
    event: env.GITHUB_EVENT_NAME ?? null,
  },
  limits: {
    cargo_build_jobs: env.CARGO_BUILD_JOBS ?? null,
    installer_budget_bytes: INSTALLER_BUDGET_BYTES,
    target_dir_budget_bytes: TARGET_DIR_BUDGET_BYTES,
  },
  measured: {
    target_dir_bytes: targetDirBytes,
  },
  artifacts: [{ name: installerName, bytes: installerBytes, sha256: sha256(installerPath) }],
});

// Last, so the checksum file covers the installer and both manifests.
const checksums = readdirSync(outputDir)
  .sort()
  .map((name) => `${sha256(join(outputDir, name))}  ${name}\n`)
  .join("");
writeFileSync(join(outputDir, "SHA256SUMS.txt"), checksums);

console.log(`Packaged ${basename(installerPath)} (${String(installerBytes)} bytes) into ${outputDir}`);
console.log(checksums.trimEnd());
