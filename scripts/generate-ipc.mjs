// Rewrites src/ipc/generated from the Rust command contract in crates/skribe-ipc.
// The same test, run without the variable below, fails when the committed files are stale.
import { spawnSync } from "node:child_process";

const result = spawnSync("cargo", ["test", "--locked", "-p", "skribe-ipc", "bindings::"], {
  stdio: "inherit",
  env: { ...process.env, SKRIBE_UPDATE_IPC: "1" },
});
process.exit(result.status ?? 1);
