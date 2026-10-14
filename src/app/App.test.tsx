import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { call, listen } from "../ipc";
import {
  corrupt,
  detail,
  idle,
  library,
  meeting,
  ok,
  ready,
  recordingDevices,
  recordingStatus,
  settings,
  unavailable,
} from "../test/fixtures";
import { answer, captureEvents } from "../test/ipc";
import type { Answers } from "../test/ipc";
import { App } from "./App";

vi.mock("../ipc", () => ({ call: vi.fn(), listen: vi.fn() }));
const callMock = vi.mocked(call);
const listenMock = vi.mocked(listen);

beforeEach(() => {
  callMock.mockReset();
  listenMock.mockReset();
});

const empty = { meetings: [], trashed: [] };

/** Renders the app with an open library and nothing being recorded, unless overridden. */
function show(answers: Answers = {}) {
  const events = captureEvents(listenMock);
  answer(callMock, {
    library_status: ok(ready),
    meetings_list: ok(empty),
    recording_status: ok(idle),
    ...answers,
  });
  render(<App />);
  return events;
}

test("says the library is opening, then shows the meeting list", async () => {
  show();

  expect(screen.getByRole("status")).toHaveTextContent("Opening your library…");
  expect(await screen.findByRole("heading", { name: "Meetings" })).toBeInTheDocument();
  expect(await screen.findByText("No meetings yet")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Meetings" })).toHaveAttribute("aria-current", "page");
});

test("reports what opening the library had to do", async () => {
  const message = "The library was updated for this version of Skribe.";
  show({
    library_status: ok({
      state: "ready",
      library: { ...library, openNotices: [{ kind: "library_updated", message }] },
    }),
  });

  expect(await screen.findByText(message)).toBeInTheDocument();
});

test("shows the recovery screen instead of the app when the library cannot be opened", async () => {
  show({ library_status: ok(unavailable(corrupt)) });

  expect(
    await screen.findByRole("heading", { name: "Skribe could not open your library" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Meetings" })).not.toBeInTheDocument();
  expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
});

test("settings show the shortcut and where data is kept", async () => {
  const user = userEvent.setup();
  show({ settings_get: ok(settings), recording_devices: ok(recordingDevices) });
  await screen.findByText("No meetings yet");

  await user.click(screen.getByRole("button", { name: "Settings" }));

  expect(await screen.findByRole("textbox", { name: "Shortcut" })).toHaveValue("Ctrl+Shift+F9");
  expect(await screen.findByRole("combobox", { name: "Microphone" })).toHaveValue("default");
  expect(screen.getByText(library.locations.library)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Settings" })).toHaveAttribute("aria-current", "page");
});

test("opening a meeting shows it, and leaving with unsaved text asks first", async () => {
  const user = userEvent.setup();
  const budget = meeting({ title: "Budget review" });
  show({
    meetings_list: ok({ meetings: [budget], trashed: [] }),
    meeting_get: ok(detail(["Intro"], { title: "Budget review" })),
  });

  await user.click(await screen.findByRole("button", { name: "Open Budget review" }));
  const title = await screen.findByRole("textbox", { name: "Title" });
  expect(screen.getByRole("button", { name: "Start recording" })).toBeInTheDocument();

  await user.type(title, " 2027");
  await user.click(screen.getByRole("button", { name: "Back to meetings" }));

  const question = screen.getByRole("group", { name: "Leave this page without saving?" });
  expect(question).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Stay on this page" })).toHaveFocus();
  expect(title).toHaveValue("Budget review 2027");

  await user.keyboard("{Enter}");
  expect(
    screen.queryByRole("group", { name: "Leave this page without saving?" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Title" })).toHaveValue("Budget review 2027");

  await user.click(screen.getByRole("button", { name: "Meetings" }));
  await user.click(screen.getByRole("button", { name: "Leave without saving" }));

  expect(await screen.findByRole("list", { name: "Meetings" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Create meeting" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Back to meetings" })).not.toBeInTheDocument();
});

test("a recording in progress is one step away from any page", async () => {
  const user = userEvent.setup();
  const budget = meeting({ title: "Budget review", recordingState: "recording" });
  const live = recordingStatus(budget.id);
  show({
    meetings_list: ok({ meetings: [budget], trashed: [] }),
    recording_status: ok(live),
    meeting_get: ok(
      detail([], { title: "Budget review", recordingState: "recording" }, { agendaEditable: false }),
    ),
  });

  expect(await screen.findByText("A recording is in progress.")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Go to the recording" }));

  expect(await screen.findByRole("list", { name: "Tracks being recorded" })).toBeInTheDocument();
  // On the recording's own page the reminder is not repeated.
  expect(screen.queryByText("A recording is in progress.")).not.toBeInTheDocument();
});

test("says why the window did not close, until the recording has stopped", async () => {
  const budget = meeting({ title: "Budget review", recordingState: "recording" });
  const events = show({
    meetings_list: ok({ meetings: [budget], trashed: [] }),
    recording_status: ok(recordingStatus(budget.id)),
  });
  await screen.findByText("A recording is in progress.");
  const message = "Skribe is recording. Stop the recording before closing the window.";

  act(() => {
    events.emit("close-blocked", { message });
  });

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent("Skribe was not closed.");
  expect(alert).toHaveTextContent(message);

  act(() => {
    events.emit("recording-status", idle);
  });

  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.queryByText("A recording is in progress.")).not.toBeInTheDocument();
});
