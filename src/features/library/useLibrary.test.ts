import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";

import { call } from "../../ipc";
import { corrupt, inUse, library, ready, snapshot, snapshotGone, unavailable } from "../../test/fixtures";
import { useLibrary } from "./useLibrary";

vi.mock("../../ipc", () => ({ call: vi.fn() }));
const callMock = vi.mocked(call);

test("reports opening until Rust says the library is ready", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: ready });

  const { result } = renderHook(() => useLibrary());

  expect(result.current.view).toEqual({ phase: "opening" });
  await waitFor(() => {
    expect(result.current.view).toEqual({ phase: "ready", library });
  });
  expect(callMock).toHaveBeenCalledWith("library_status");
});

test("a library that cannot be opened is unavailable with its snapshots", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(corrupt, [snapshot]) });

  const { result } = renderHook(() => useLibrary());

  await waitFor(() => {
    expect(result.current.view).toEqual({
      phase: "unavailable",
      error: corrupt,
      snapshots: [snapshot],
      pending: null,
      recoveryError: null,
    });
  });
});

test("a status call that fails outright is shown as unavailable, not as opening forever", async () => {
  const error = { ...inUse, code: "internal" as const, actions: [] };
  callMock.mockResolvedValueOnce({ ok: false, error });

  const { result } = renderHook(() => useLibrary());

  await waitFor(() => {
    expect(result.current.view).toMatchObject({ phase: "unavailable", error, snapshots: [] });
  });
});

test("retry shows progress, then the library once it opens", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(inUse) });
  const { result } = renderHook(() => useLibrary());
  await waitFor(() => {
    expect(result.current.view.phase).toBe("unavailable");
  });

  let finishRetry: (status: typeof ready) => void = () => undefined;
  callMock.mockReturnValueOnce(
    new Promise((resolve) => {
      finishRetry = (value) => {
        resolve({ ok: true, value });
      };
    }),
  );
  act(() => {
    result.current.retry();
  });

  expect(callMock).toHaveBeenLastCalledWith("library_retry_open");
  expect(result.current.view).toMatchObject({ phase: "unavailable", pending: "retry" });

  act(() => {
    finishRetry(ready);
  });
  await waitFor(() => {
    expect(result.current.view.phase).toBe("ready");
  });
});

test("a retry that fails again replaces the reason with the new one", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(inUse) });
  const { result } = renderHook(() => useLibrary());
  await waitFor(() => {
    expect(result.current.view.phase).toBe("unavailable");
  });

  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(corrupt, [snapshot]) });
  act(() => {
    result.current.retry();
  });

  await waitFor(() => {
    expect(result.current.view).toMatchObject({ error: corrupt, snapshots: [snapshot] });
  });
});

test("a restore that does not run keeps the original failure and adds its own", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(corrupt, [snapshot]) });
  const { result } = renderHook(() => useLibrary());
  await waitFor(() => {
    expect(result.current.view.phase).toBe("unavailable");
  });

  callMock.mockResolvedValueOnce({ ok: false, error: snapshotGone });
  act(() => {
    result.current.restoreSnapshot(snapshot.name);
  });

  expect(callMock).toHaveBeenLastCalledWith("library_restore_snapshot", {
    snapshotName: snapshot.name,
  });
  await waitFor(() => {
    expect(result.current.view).toEqual({
      phase: "unavailable",
      error: corrupt,
      snapshots: [snapshot],
      pending: null,
      recoveryError: snapshotGone,
    });
  });
});

test("a successful restore opens the library", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(corrupt, [snapshot]) });
  const { result } = renderHook(() => useLibrary());
  await waitFor(() => {
    expect(result.current.view.phase).toBe("unavailable");
  });

  callMock.mockResolvedValueOnce({ ok: true, value: ready });
  act(() => {
    result.current.restoreSnapshot(snapshot.name);
  });

  await waitFor(() => {
    expect(result.current.view).toEqual({ phase: "ready", library });
  });
});
