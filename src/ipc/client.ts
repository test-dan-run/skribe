import { invoke } from "@tauri-apps/api/core";

import type { CommandError, Commands } from "./generated";

type CommandName = keyof Commands;
type Args<K extends CommandName> = Commands[K]["args"];
type Value<K extends CommandName> = Commands[K]["result"];
type Parameters<K extends CommandName> = Args<K> extends null ? [] : [args: Args<K>];

/** The outcome of a command. Failures are values, so callers cannot forget to handle them. */
export type CommandResult<T> = { ok: true; value: T } | { ok: false; error: CommandError };

/**
 * Calls a Skribe command. This is the only place the frontend talks to Tauri; command names,
 * arguments and results are checked against the types generated from Rust.
 */
export async function call<K extends CommandName>(
  name: K,
  ...parameters: Parameters<K>
): Promise<CommandResult<Value<K>>> {
  try {
    const args = parameters.length === 0 ? undefined : { args: parameters[0] };
    return { ok: true, value: await invoke<Value<K>>(name, args) };
  } catch (reason) {
    return { ok: false, error: isCommandError(reason) ? reason : unreachable(reason) };
  }
}

function isCommandError(value: unknown): value is CommandError {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate["code"] === "string" &&
    typeof candidate["message"] === "string" &&
    typeof candidate["retained"] === "string" &&
    typeof candidate["recovery"] === "string" &&
    Array.isArray(candidate["actions"])
  );
}

/** The command never reported back in the agreed shape, so its outcome is unknown. */
function unreachable(reason: unknown): CommandError {
  return {
    code: "internal",
    message: "Skribe's window could not reach the rest of the app.",
    retained:
      "Skribe could not confirm what this request did. Data already saved stays in your library.",
    recovery: "Restart Skribe.",
    actions: [],
    detail: reason instanceof Error ? reason.message : String(reason),
  };
}
