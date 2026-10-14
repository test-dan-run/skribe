import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { call } from "../../ipc";
import { detail, failed, inUse, invalidTitle, meeting, ok } from "../../test/fixtures";
import { MeetingsHome } from "./MeetingsHome";

vi.mock("../../ipc", () => ({ call: vi.fn() }));
const callMock = vi.mocked(call);

beforeEach(() => {
  callMock.mockReset();
});

const empty = { meetings: [], trashed: [] };
const budget = meeting({ id: "m-budget", title: "Budget review", agendaItemCount: 3 });
const hiring = meeting({ id: "m-hiring", title: "Hiring", agendaItemCount: 1 });
const old = meeting({
  id: "m-old",
  title: "Old plan",
  trashedAtUnixMs: 1_791_234_999_000,
});

test("says the list is loading, then that there are no meetings", async () => {
  callMock.mockResolvedValueOnce(ok(empty));

  render(<MeetingsHome onOpen={vi.fn()} />);

  expect(screen.getByRole("status")).toHaveTextContent("Loading your meetings…");
  expect(await screen.findByText("No meetings yet")).toBeInTheDocument();
  expect(screen.getByText("The trash is empty.")).toBeInTheDocument();
  expect(callMock).toHaveBeenCalledWith("meetings_list");
});

test("a list that cannot be loaded is explained and can be tried again", async () => {
  const user = userEvent.setup();
  callMock.mockResolvedValueOnce(failed(inUse));
  render(<MeetingsHome onOpen={vi.fn()} />);

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(inUse.message);
  expect(screen.queryByText("No meetings yet")).not.toBeInTheDocument();

  callMock.mockResolvedValueOnce(ok({ meetings: [budget], trashed: [] }));
  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(await screen.findByText("Budget review")).toBeInTheDocument();
});

test("lists meetings with their plan and state, and opens one", async () => {
  const user = userEvent.setup();
  const onOpen = vi.fn();
  callMock.mockResolvedValueOnce(ok({ meetings: [budget, hiring], trashed: [] }));
  render(<MeetingsHome onOpen={onOpen} />);

  const list = await screen.findByRole("list", { name: "Meetings" });
  const rows = within(list).getAllByRole("listitem");
  expect(rows).toHaveLength(2);
  expect(rows[0]).toHaveTextContent("Budget review");
  expect(rows[0]).toHaveTextContent("No planned time · 3 agenda items · Not recorded yet");
  expect(rows[1]).toHaveTextContent("1 agenda item ·");

  await user.click(screen.getByRole("button", { name: "Open Hiring" }));

  expect(onOpen).toHaveBeenCalledExactlyOnceWith("m-hiring");
});

test("creating a meeting from the keyboard opens it", async () => {
  const user = userEvent.setup();
  const onOpen = vi.fn();
  callMock.mockResolvedValueOnce(ok(empty));
  render(<MeetingsHome onOpen={onOpen} />);
  const title = await screen.findByRole("textbox", { name: "Title" });

  callMock.mockResolvedValueOnce(ok(detail([], { id: "m-new", title: "Quarterly review" })));
  await user.type(title, "Quarterly review{Enter}");

  expect(callMock).toHaveBeenLastCalledWith("meeting_create", {
    title: "Quarterly review",
    plannedStartUnixMs: null,
  });
  expect(onOpen).toHaveBeenCalledExactlyOnceWith("m-new");
});

test("a meeting that could not be created is explained and the typed title is kept", async () => {
  const user = userEvent.setup();
  const onOpen = vi.fn();
  callMock.mockResolvedValueOnce(ok(empty));
  render(<MeetingsHome onOpen={onOpen} />);
  const title = await screen.findByRole("textbox", { name: "Title" });

  callMock.mockResolvedValueOnce(failed(invalidTitle));
  await user.type(title, "   ");
  await user.click(screen.getByRole("button", { name: "Create meeting" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Give the meeting a title.");
  expect(title).toHaveValue("   ");
  expect(onOpen).not.toHaveBeenCalled();
});

test("moving a meeting to the trash asks first and says nothing is deleted", async () => {
  const user = userEvent.setup();
  callMock.mockResolvedValueOnce(ok({ meetings: [budget], trashed: [] }));
  render(<MeetingsHome onOpen={vi.fn()} />);

  await user.click(await screen.findByRole("button", { name: "Move Budget review to the trash" }));

  const question = screen.getByRole("group", { name: "Move “Budget review” to the trash?" });
  expect(question).toHaveTextContent("Nothing is deleted.");
  expect(callMock).toHaveBeenCalledTimes(1);

  callMock.mockResolvedValueOnce(
    ok({ meetings: [], trashed: [{ ...budget, trashedAtUnixMs: 1_791_234_999_000 }] }),
  );
  await user.click(within(question).getByRole("button", { name: "Move to trash" }));

  expect(callMock).toHaveBeenLastCalledWith("meeting_trash", { meetingId: "m-budget" });
  const trash = await screen.findByRole("list", { name: "Meetings in the trash" });
  expect(trash).toHaveTextContent("Budget review");
  expect(screen.getByText("No meetings yet")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Meeting moved to the trash.");
});

test("a meeting in the trash can be restored", async () => {
  const user = userEvent.setup();
  callMock.mockResolvedValueOnce(ok({ meetings: [], trashed: [old] }));
  render(<MeetingsHome onOpen={vi.fn()} />);

  callMock.mockResolvedValueOnce(
    ok({ meetings: [{ ...old, trashedAtUnixMs: null }], trashed: [] }),
  );
  await user.click(await screen.findByRole("button", { name: "Restore Old plan" }));

  expect(callMock).toHaveBeenLastCalledWith("meeting_restore", { meetingId: "m-old" });
  expect(await screen.findByRole("list", { name: "Meetings" })).toHaveTextContent("Old plan");
  expect(screen.getByText("The trash is empty.")).toBeInTheDocument();
});

test("permanent deletion is a separate confirmation that says it cannot be undone", async () => {
  const user = userEvent.setup();
  const recorded = { ...old, recordingState: "recorded" as const };
  callMock.mockResolvedValueOnce(ok({ meetings: [], trashed: [recorded] }));
  render(<MeetingsHome onOpen={vi.fn()} />);

  await user.click(await screen.findByRole("button", { name: "Permanently delete Old plan" }));

  const question = screen.getByRole("group", { name: "Permanently delete “Old plan”?" });
  expect(question).toHaveTextContent("This cannot be undone.");
  expect(question).toHaveTextContent("together with its audio files");
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  expect(callMock).toHaveBeenCalledTimes(1);

  callMock.mockResolvedValueOnce(ok(empty));
  await user.click(within(question).getByRole("button", { name: "Delete permanently" }));

  expect(callMock).toHaveBeenLastCalledWith("meeting_delete_permanently", { meetingId: "m-old" });
  expect(await screen.findByText("The trash is empty.")).toBeInTheDocument();
});

test("a change that fails leaves the list as it was and says why", async () => {
  const user = userEvent.setup();
  callMock.mockResolvedValueOnce(ok({ meetings: [], trashed: [old] }));
  render(<MeetingsHome onOpen={vi.fn()} />);

  callMock.mockResolvedValueOnce(failed(inUse));
  await user.click(await screen.findByRole("button", { name: "Restore Old plan" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(inUse.message);
  expect(screen.getByRole("list", { name: "Meetings in the trash" })).toHaveTextContent("Old plan");
});
