import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { call } from "../../ipc";
import type { CommandError, RecordingDevices } from "../../ipc";
import { conflict, failed, ok, recordingDevices } from "../../test/fixtures";
import { answer } from "../../test/ipc";
import { RecordingDevicesForm } from "./RecordingDevicesForm";

vi.mock("../../ipc", () => ({ call: vi.fn() }));
const callMock = vi.mocked(call);

beforeEach(() => {
  callMock.mockReset();
});

const captureUnavailable: CommandError = {
  code: "capture_unavailable",
  message: "This build of Skribe cannot record audio.",
  retained: "Nothing was recorded and the meeting was not changed.",
  recovery: "Recording needs Skribe for Windows.",
  actions: [],
  detail: null,
};

async function show(devices: RecordingDevices = recordingDevices) {
  answer(callMock, { recording_devices: ok(devices) });
  const onDirtyChange = vi.fn();
  render(<RecordingDevicesForm onDirtyChange={onDirtyChange} />);
  const microphone = await screen.findByRole("combobox", { name: "Microphone" });
  const system = screen.getByRole("combobox", { name: "System audio (playback device)" });
  return { microphone, system, onDirtyChange };
}

const optionNames = (select: HTMLElement) =>
  within(select)
    .getAllByRole("option")
    .map((option) => option.textContent);

test("lists the Windows defaults by name and every connected device", async () => {
  const { microphone, system } = await show();

  expect(microphone).toHaveValue("default");
  expect(optionNames(microphone)).toEqual([
    "Windows default device (currently Laptop Microphone)",
    "Windows default communications device (currently Headset Microphone)",
    "Laptop Microphone",
    "Headset Microphone",
  ]);
  expect(optionNames(system)).toEqual([
    "Windows default device (currently Speakers)",
    "Windows default communications device (currently Headset Earphones)",
    "Speakers",
    "Headset Earphones",
  ]);
  expect(screen.getByText(/default communications device, which can be a different device/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save devices" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("choosing devices from the keyboard and saving sends identifiers, not names", async () => {
  const user = userEvent.setup();
  const { microphone, system, onDirtyChange } = await show();

  await user.selectOptions(microphone, "Headset Microphone");
  await user.selectOptions(
    system,
    "Windows default communications device (currently Headset Earphones)",
  );
  expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);

  const saved: RecordingDevices = {
    ...recordingDevices,
    revision: 1,
    microphone: {
      ...recordingDevices.microphone,
      choice: { kind: "device", id: "mic-headset", name: "Headset Microphone" },
    },
    system: { ...recordingDevices.system, choice: { kind: "communications" } },
  };
  answer(callMock, { recording_set_devices: ok(saved) });
  screen.getByRole("button", { name: "Save devices" }).focus();
  await user.keyboard("{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("recording_set_devices", {
    microphone: { kind: "device", deviceId: "mic-headset" },
    system: { kind: "communications" },
    expectedRevision: 0,
  });
  expect(await screen.findByText("Recording devices saved.")).toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Microphone" })).toHaveValue("device:mic-headset");
  expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();
  await waitFor(() => {
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  });
});

test("a chosen device that is unplugged stays chosen, and says recording will not start", async () => {
  const { system } = await show({
    ...recordingDevices,
    revision: 3,
    system: {
      ...recordingDevices.system,
      choice: { kind: "device", id: "out-dock", name: "Dock Audio" },
      choiceAvailable: false,
    },
  });

  expect(system).toHaveValue("device:out-dock");
  expect(optionNames(system)).toContain("Dock Audio (not connected)");
  expect(system).toHaveAccessibleDescription(/Recording will not start until/);
  expect(system).toHaveAccessibleDescription(/does not switch devices on its own/);
  // The other track is unaffected.
  expect(screen.getByRole("combobox", { name: "Microphone" })).toHaveAccessibleDescription("");
});

test("a save Rust refuses is explained and the choice on screen is kept", async () => {
  const user = userEvent.setup();
  const { microphone } = await show();

  await user.selectOptions(microphone, "Headset Microphone");
  answer(callMock, { recording_set_devices: failed(conflict) });
  await user.click(screen.getByRole("button", { name: "Save devices" }));

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(conflict.message);
  expect(microphone).toHaveValue("device:mic-headset");

  const current: RecordingDevices = { ...recordingDevices, revision: 1 };
  answer(callMock, { recording_devices: ok(current) });
  await user.click(within(alert).getByRole("button", { name: "Load the saved version" }));

  await waitFor(() => {
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
  expect(screen.getByRole("combobox", { name: "Microphone" })).toHaveValue("default");
});

test("refreshing picks up a device that was just plugged in", async () => {
  const user = userEvent.setup();
  const { microphone } = await show();
  expect(optionNames(microphone)).toHaveLength(4);

  answer(callMock, {
    recording_devices: ok({
      ...recordingDevices,
      microphone: {
        ...recordingDevices.microphone,
        available: [
          ...recordingDevices.microphone.available,
          { id: "mic-usb", name: "USB Microphone", isDefault: false, isCommunicationsDefault: false },
        ],
      },
    }),
  });
  await user.click(screen.getByRole("button", { name: "Refresh the list" }));

  expect(await screen.findByRole("option", { name: "USB Microphone" })).toBeInTheDocument();
  expect(screen.getByText("Device list refreshed.")).toBeInTheDocument();
});

test("a build that cannot record says so instead of listing devices", async () => {
  const none = {
    choice: { kind: "default" as const },
    choiceAvailable: false,
    available: [],
    unavailable: captureUnavailable,
  };
  const { microphone } = await show({ ...recordingDevices, microphone: none, system: none });

  expect(optionNames(microphone)).toEqual([
    "Windows default device (none connected)",
    "Windows default communications device (none connected)",
  ]);
  expect(microphone).toHaveAccessibleDescription(/This build of Skribe cannot record audio/);
});

test("a development build says its devices are test tones", async () => {
  await show({ ...recordingDevices, synthetic: true });

  expect(screen.getByText(/generated test tones, not your real devices/)).toBeInTheDocument();
});

test("a list that cannot be loaded is explained and can be tried again", async () => {
  const user = userEvent.setup();
  answer(callMock, { recording_devices: failed(conflict) });
  render(<RecordingDevicesForm onDirtyChange={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(conflict.message);

  answer(callMock, { recording_devices: ok(recordingDevices) });
  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(await screen.findByRole("combobox", { name: "Microphone" })).toBeInTheDocument();
});
