import { listen as tauriListen } from "@tauri-apps/api/event";
import { expect, test, vi } from "vitest";

import { listen } from "./events";
import type { RecordingStatus } from "./generated";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
const tauriListenMock = vi.mocked(tauriListen);

const idle: RecordingStatus = { phase: "idle" };

type Callback = (event: { payload: RecordingStatus }) => void;

test("delivers the payload of the named event until stopped", async () => {
  const unlisten = vi.fn();
  let deliver: Callback = () => undefined;
  tauriListenMock.mockImplementationOnce((_name, callback) => {
    deliver = callback as unknown as Callback;
    return Promise.resolve(unlisten);
  });
  const handler = vi.fn();

  const stop = listen("recording-status", handler);
  await Promise.resolve();
  deliver({ payload: idle });

  expect(tauriListenMock).toHaveBeenCalledWith("recording-status", expect.any(Function));
  expect(handler).toHaveBeenCalledExactlyOnceWith(idle);

  stop();
  deliver({ payload: idle });

  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(handler).toHaveBeenCalledTimes(1);
});

test("stopping before registration finishes still unregisters", async () => {
  const unlisten = vi.fn();
  let finish: (stop: () => void) => void = () => undefined;
  tauriListenMock.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );

  const stop = listen("recording-status", vi.fn());
  stop();
  finish(unlisten);
  await Promise.resolve();
  await Promise.resolve();

  expect(unlisten).toHaveBeenCalledTimes(1);
});

test("a window with nothing to listen to does not throw", async () => {
  tauriListenMock.mockRejectedValueOnce(new TypeError("no IPC"));

  const stop = listen("recording-status", vi.fn());
  await Promise.resolve();
  await Promise.resolve();

  expect(() => {
    stop();
  }).not.toThrow();
});
