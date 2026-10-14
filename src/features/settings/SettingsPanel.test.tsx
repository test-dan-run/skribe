import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { call } from "../../ipc";
import type { CommandError } from "../../ipc";
import { failed, ok, recordingDevices, settings } from "../../test/fixtures";
import { answer } from "../../test/ipc";
import type { Answers } from "../../test/ipc";
import { SettingsPanel } from "./SettingsPanel";
import { shortcutFromKeyPress } from "./shortcutKeys";

vi.mock("../../ipc", () => ({ call: vi.fn() }));
const callMock = vi.mocked(call);

beforeEach(() => {
  callMock.mockReset();
});

const needsModifier: CommandError = {
  code: "invalid_input",
  message:
    "The shortcut works while other apps have focus, so it must include Ctrl, Alt or the Windows key.",
  retained: "Nothing was changed.",
  recovery: "Check the value and try again.",
  actions: [],
  detail: null,
};

/** Answers the two commands the settings page loads with, plus whatever a test adds. */
function loadWith(answers: Answers = {}) {
  answer(callMock, {
    settings_get: ok(settings),
    recording_devices: ok(recordingDevices),
    ...answers,
  });
}

async function show(answers: Answers = {}) {
  loadWith(answers);
  const onDirtyChange = vi.fn();
  render(<SettingsPanel onDirtyChange={onDirtyChange} />);
  const input = await screen.findByRole("textbox", { name: "Shortcut" });
  return { input, onDirtyChange };
}

test("shows the saved shortcut and says it is not used yet", async () => {
  const { input } = await show();

  expect(input).toHaveValue("Ctrl+Shift+F9");
  expect(screen.getByText(/does not act on it yet/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save shortcut" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("pressing a combination in the field captures it, and saving stores it", async () => {
  const user = userEvent.setup();
  const { input, onDirtyChange } = await show();

  input.focus();
  await user.keyboard("{Control>}{Alt>}n{/Alt}{/Control}");
  expect(input).toHaveValue("Ctrl+Alt+N");
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);

  loadWith({
    settings_set_section_shortcut: ok({
      ...settings,
      sectionShortcut: "Ctrl+Alt+N",
      sectionShortcutRevision: 1,
    }),
  });
  await user.click(screen.getByRole("button", { name: "Save shortcut" }));

  expect(callMock).toHaveBeenLastCalledWith("settings_set_section_shortcut", {
    shortcut: "Ctrl+Alt+N",
    expectedRevision: 0,
  });
  expect(await screen.findByText("Shortcut saved as Ctrl+Alt+N.")).toBeInTheDocument();
  await waitFor(() => {
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  });
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
});

test("a shortcut Rust refuses is explained and the typed text is kept", async () => {
  const user = userEvent.setup();
  const { input } = await show();

  await user.clear(input);
  loadWith({ settings_set_section_shortcut: failed(needsModifier) });
  await user.type(input, "Shift+N{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("settings_set_section_shortcut", {
    shortcut: "Shift+N",
    expectedRevision: 0,
  });
  expect(await screen.findByRole("alert")).toHaveTextContent("must include Ctrl, Alt");
  expect(input).toHaveValue("Shift+N");
});

test("the default can be put back", async () => {
  const user = userEvent.setup();
  loadWith({
    settings_get: ok({ ...settings, sectionShortcut: "Ctrl+Alt+N", sectionShortcutRevision: 1 }),
  });
  render(<SettingsPanel onDirtyChange={vi.fn()} />);
  const input = await screen.findByRole("textbox", { name: "Shortcut" });

  await user.click(screen.getByRole("button", { name: "Use the default (Ctrl+Shift+F9)" }));

  expect(input).toHaveValue("Ctrl+Shift+F9");
  expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
});

test("Tab still leaves the field", async () => {
  const user = userEvent.setup();
  const { input } = await show();

  input.focus();
  await user.tab();

  expect(input).not.toHaveFocus();
  expect(input).toHaveValue("Ctrl+Shift+F9");
});

test("only combinations with Ctrl, Alt or the Windows key are captured", () => {
  const press = (code: string, held: string[] = []) => ({
    code,
    ctrlKey: held.includes("ctrl"),
    altKey: held.includes("alt"),
    shiftKey: held.includes("shift"),
    metaKey: held.includes("meta"),
  });

  expect(shortcutFromKeyPress(press("F9", ["ctrl", "shift"]))).toBe("Ctrl+Shift+F9");
  expect(shortcutFromKeyPress(press("Digit7", ["meta"]))).toBe("Super+7");
  expect(shortcutFromKeyPress(press("KeyZ", ["ctrl", "alt", "shift", "meta"]))).toBe(
    "Ctrl+Alt+Shift+Super+Z",
  );
  expect(shortcutFromKeyPress(press("F24", ["alt"]))).toBe("Alt+F24");
  // Ordinary typing, navigation and keys a shortcut cannot use are left alone.
  expect(shortcutFromKeyPress(press("KeyN"))).toBeNull();
  expect(shortcutFromKeyPress(press("KeyN", ["shift"]))).toBeNull();
  expect(shortcutFromKeyPress(press("Tab", ["ctrl"]))).toBeNull();
  expect(shortcutFromKeyPress(press("F25", ["ctrl"]))).toBeNull();
  expect(shortcutFromKeyPress(press("ControlLeft", ["ctrl"]))).toBeNull();
});
