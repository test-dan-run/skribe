import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { call } from "../../ipc";
import { conflict, detail, failed, inUse, ok } from "../../test/fixtures";
import { PrepareMeeting } from "./PrepareMeeting";
import { fromInputValue, toInputValue } from "./plannedTime";

vi.mock("../../ipc", () => ({ call: vi.fn() }));
const callMock = vi.mocked(call);

beforeEach(() => {
  callMock.mockReset();
});

const meetingId = detail().meeting.id;
const target = { meetingId, expectedRevision: 4 };

function show(loaded = detail(["Intro", "Budget", "Hiring"])) {
  callMock.mockResolvedValueOnce(ok(loaded));
  const props = { onBack: vi.fn(), onDirtyChange: vi.fn() };
  render(<PrepareMeeting meetingId={meetingId} {...props} />);
  return props;
}

const agendaTitles = () =>
  within(screen.getByRole("list"))
    .getAllByRole("listitem")
    .map((row) => row.textContent);

test("loads the meeting and shows its details and agenda in order", async () => {
  show();

  expect(screen.getByRole("status")).toHaveTextContent("Loading the meeting…");
  expect(await screen.findByRole("heading", { name: "Weekly sync" })).toBeInTheDocument();
  expect(callMock).toHaveBeenCalledWith("meeting_get", { meetingId });
  expect(screen.getByRole("textbox", { name: "Title" })).toHaveValue("Weekly sync");
  expect(screen.getByText("Not recorded yet")).toBeInTheDocument();
  const rows = within(screen.getByRole("list")).getAllByRole("listitem");
  expect(rows.map((row) => row.textContent)).toEqual([
    expect.stringContaining("1. Intro"),
    expect.stringContaining("2. Budget"),
    expect.stringContaining("3. Hiring"),
  ]);
});

test("a meeting that cannot be loaded is explained, with a way back", async () => {
  const user = userEvent.setup();
  callMock.mockResolvedValueOnce(failed(inUse));
  const onBack = vi.fn();
  render(<PrepareMeeting meetingId={meetingId} onBack={onBack} onDirtyChange={vi.fn()} />);

  expect(await screen.findByRole("alert")).toHaveTextContent(inUse.message);
  await user.click(screen.getByRole("button", { name: "Back to meetings" }));

  expect(onBack).toHaveBeenCalledTimes(1);
});

test("saving the details sends the revision that was loaded", async () => {
  const user = userEvent.setup();
  const props = show();
  const title = await screen.findByRole("textbox", { name: "Title" });
  const save = screen.getByRole("button", { name: "Save details" });
  expect(save).toHaveAttribute("aria-disabled", "true");

  await user.clear(title);
  await user.type(title, "Weekly sync (moved)");
  const planned = screen.getByLabelText("Planned time (optional)");
  fireEvent.change(planned, { target: { value: "2027-03-04T09:30" } });
  expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
  expect(props.onDirtyChange).toHaveBeenLastCalledWith(true);

  const plannedStartUnixMs = new Date(2027, 2, 4, 9, 30).getTime();
  callMock.mockResolvedValueOnce(
    ok(
      detail(["Intro", "Budget", "Hiring"], {
        title: "Weekly sync (moved)",
        plannedStartUnixMs,
        revision: 5,
      }),
    ),
  );
  await user.click(save);

  expect(callMock).toHaveBeenLastCalledWith("meeting_update", {
    ...target,
    title: "Weekly sync (moved)",
    plannedStartUnixMs,
  });
  expect(await screen.findByRole("heading", { name: "Weekly sync (moved)" })).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Meeting details saved.");
  expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();
  await waitFor(() => {
    expect(props.onDirtyChange).toHaveBeenLastCalledWith(false);
  });
});

test("agenda items are added from the keyboard without leaving the field", async () => {
  const user = userEvent.setup();
  show(detail([]));
  const input = await screen.findByRole("textbox", { name: "New agenda item" });
  expect(screen.getByText(/No agenda items yet/)).toBeInTheDocument();

  callMock.mockResolvedValueOnce(ok(detail(["Intro"], { revision: 5 })));
  await user.type(input, "Intro{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("agenda_add", { ...target, title: "Intro" });
  await waitFor(() => {
    expect(input).toHaveValue("");
  });
  expect(input).toHaveFocus();
  expect(agendaTitles()).toEqual([expect.stringContaining("1. Intro")]);
  expect(screen.getByRole("status")).toHaveTextContent("Added as item 1 of 1.");

  callMock.mockResolvedValueOnce(ok(detail(["Intro", "Budget"], { revision: 6 })));
  await user.type(input, "Budget{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("agenda_add", {
    meetingId,
    expectedRevision: 5,
    title: "Budget",
  });
});

test("an item is moved from the keyboard, saved as one new order, and keeps focus", async () => {
  const user = userEvent.setup();
  show();
  const down = await screen.findByRole("button", { name: "Move Intro down" });
  expect(screen.getByRole("button", { name: "Move Intro up" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByRole("button", { name: "Move Hiring down" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );

  callMock.mockResolvedValueOnce(ok(detail(["Budget", "Intro", "Hiring"], { revision: 5 })));
  down.focus();
  await user.keyboard("{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("agenda_reorder", {
    ...target,
    itemIds: ["item-Budget", "item-Intro", "item-Hiring"],
  });
  await waitFor(() => {
    expect(screen.getByRole("status")).toHaveTextContent("Moved to position 2 of 3.");
  });
  expect(agendaTitles()).toEqual([
    expect.stringContaining("1. Budget"),
    expect.stringContaining("2. Intro"),
    expect.stringContaining("3. Hiring"),
  ]);
  expect(screen.getByRole("button", { name: "Move Intro down" })).toHaveFocus();

  // The first item cannot move up: nothing is sent.
  await user.click(screen.getByRole("button", { name: "Move Budget up" }));
  expect(callMock).toHaveBeenCalledTimes(2);
});

test("renaming an item saves on Enter and Escape cancels back to the button", async () => {
  const user = userEvent.setup();
  show();

  await user.click(await screen.findByRole("button", { name: "Rename Budget" }));
  const input = screen.getByRole("textbox", { name: "Agenda item title" });
  expect(input).toHaveFocus();
  expect(input).toHaveValue("Budget");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("textbox", { name: "Agenda item title" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Rename Budget" })).toHaveFocus();
  expect(callMock).toHaveBeenCalledTimes(1);

  await user.keyboard("{Enter}");
  callMock.mockResolvedValueOnce(ok(detail(["Intro", "Budget 2027", "Hiring"], { revision: 5 })));
  // The field opens with its text selected, so typing replaces it.
  await user.keyboard("Budget 2027{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("agenda_edit", {
    ...target,
    itemId: "item-Budget",
    title: "Budget 2027",
  });
  await waitFor(() => {
    expect(screen.getByRole("status")).toHaveTextContent("Agenda item renamed.");
  });
});

test("removing an item asks first", async () => {
  const user = userEvent.setup();
  show();

  await user.click(await screen.findByRole("button", { name: "Remove Budget" }));
  const question = screen.getByRole("group", { name: "Remove “Budget” from the agenda?" });
  expect(callMock).toHaveBeenCalledTimes(1);

  callMock.mockResolvedValueOnce(ok(detail(["Intro", "Hiring"], { revision: 5 })));
  await user.click(within(question).getByRole("button", { name: "Remove item" }));

  expect(callMock).toHaveBeenLastCalledWith("agenda_remove", { ...target, itemId: "item-Budget" });
  await waitFor(() => {
    expect(agendaTitles()).toHaveLength(2);
  });
});

test("a conflicting change is reported, keeps what is on screen, and offers the saved version", async () => {
  const user = userEvent.setup();
  show();
  const input = await screen.findByRole("textbox", { name: "New agenda item" });

  callMock.mockResolvedValueOnce(failed(conflict));
  await user.type(input, "Actions{Enter}");

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(conflict.message);
  expect(alert).toHaveTextContent("The saved version was kept.");
  // Nothing on screen was replaced or lost.
  expect(input).toHaveValue("Actions");
  expect(agendaTitles()).toHaveLength(3);

  callMock.mockResolvedValueOnce(ok(detail(["Intro", "Budget", "Hiring", "Risks"], { revision: 5 })));
  await user.click(within(alert).getByRole("button", { name: "Load the saved version" }));

  expect(callMock).toHaveBeenLastCalledWith("meeting_get", { meetingId });
  await waitFor(() => {
    expect(agendaTitles()).toHaveLength(4);
  });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();

  callMock.mockResolvedValueOnce(
    ok(detail(["Intro", "Budget", "Hiring", "Risks", "Actions"], { revision: 6 })),
  );
  await user.click(screen.getByRole("button", { name: "Add item" }));
  expect(callMock).toHaveBeenLastCalledWith("agenda_add", {
    meetingId,
    expectedRevision: 5,
    title: "Actions",
  });
});

test("once recording has started the agenda is shown but cannot be changed", async () => {
  show(detail(["Intro", "Budget"], { recordingState: "recorded" }, { agendaEditable: false }));

  expect(await screen.findByText(/its agenda can no longer be changed/)).toBeInTheDocument();
  expect(agendaTitles()).toHaveLength(2);
  expect(screen.queryByRole("button", { name: /^Move / })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^Rename / })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^Remove / })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "New agenda item" })).not.toBeInTheDocument();
  // The title stays editable.
  expect(screen.getByRole("button", { name: "Save details" })).toBeInTheDocument();
});

test("text typed for a new agenda item counts as unsaved", async () => {
  const user = userEvent.setup();
  const props = show();
  const input = await screen.findByRole("textbox", { name: "New agenda item" });

  await user.type(input, "Risks");

  expect(props.onDirtyChange).toHaveBeenLastCalledWith(true);
});

test("planned times round-trip through the date and time field in local time", () => {
  const unixMs = new Date(2027, 2, 4, 9, 5).getTime();

  expect(toInputValue(unixMs)).toBe("2027-03-04T09:05");
  expect(fromInputValue("2027-03-04T09:05")).toBe(unixMs);
  expect(toInputValue(null)).toBe("");
  expect(fromInputValue("")).toBeNull();
  expect(fromInputValue("not a date")).toBe("invalid");
});
