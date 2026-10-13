import { invoke } from "@tauri-apps/api/core";
import { expect, test, vi } from "vitest";

import { call } from "./client";
import type { CommandError, LibraryStatus } from "./generated";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

const unavailable: LibraryStatus = {
  state: "unavailable",
  error: {
    code: "library_in_use",
    message: "Your library is already open in another Skribe window.",
    retained: "Nothing was changed.",
    recovery: "Close the other Skribe window, then try again.",
    actions: ["retry_open"],
    detail: null,
  },
  snapshots: [],
};

test("a command without arguments is invoked by name alone", async () => {
  invokeMock.mockResolvedValueOnce(unavailable);

  const result = await call("library_status");

  expect(invokeMock).toHaveBeenCalledWith("library_status", undefined);
  expect(result).toEqual({ ok: true, value: unavailable });
});

test("arguments are sent as the single args parameter the Rust handlers take", async () => {
  invokeMock.mockResolvedValueOnce(unavailable);

  await call("library_restore_snapshot", { snapshotName: "skribe-v1-1.sqlite" });

  expect(invokeMock).toHaveBeenCalledWith("library_restore_snapshot", {
    args: { snapshotName: "skribe-v1-1.sqlite" },
  });
});

test("a typed error from Rust is returned unchanged", async () => {
  const error: CommandError = {
    code: "snapshot_not_found",
    message: "That snapshot is no longer in the library's backups folder.",
    retained: "Nothing was changed.",
    recovery: "Choose a different snapshot.",
    actions: [],
    detail: "snapshot skribe-v1-1.sqlite was not found",
  };
  invokeMock.mockRejectedValueOnce(error);

  const result = await call("library_restore_snapshot", { snapshotName: "skribe-v1-1.sqlite" });

  expect(result).toEqual({ ok: false, error });
});

test("any other failure becomes an internal error that does not claim an outcome", async () => {
  invokeMock.mockRejectedValueOnce(new TypeError("window.__TAURI_INTERNALS__ is undefined"));

  const result = await call("library_status");

  expect(result.ok).toBe(false);
  if (!result.ok) {
    expect(result.error.code).toBe("internal");
    expect(result.error.retained).toMatch(/could not confirm/);
    expect(result.error.actions).toEqual([]);
    expect(result.error.detail).toBe("window.__TAURI_INTERNALS__ is undefined");
  }
});
