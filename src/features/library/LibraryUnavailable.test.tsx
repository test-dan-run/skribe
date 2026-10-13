import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { expect, test, vi } from "vitest";

import { corrupt, inUse, snapshot, snapshotGone } from "../../test/fixtures";
import { LibraryUnavailable } from "./LibraryUnavailable";

type Props = ComponentProps<typeof LibraryUnavailable>;

function show(overrides: Partial<Props> = {}) {
  const props: Props = {
    error: corrupt,
    snapshots: [snapshot],
    pending: null,
    recoveryError: null,
    onRetry: vi.fn(),
    onRestore: vi.fn(),
    ...overrides,
  };
  render(<LibraryUnavailable {...props} />);
  return props;
}

const restoreTrigger = () => screen.getByRole("button", { name: /^Restore the snapshot from/ });
// <details> is also a group, so the confirmation is found by its question.
const confirmation = () => screen.queryByRole("group", { name: /^Replace the library/ });

test("says what happened, what was kept and what can be done", () => {
  show();

  expect(
    screen.getByRole("heading", { name: "Skribe could not open your library" }),
  ).toBeInTheDocument();
  expect(screen.getByText(corrupt.message)).toBeInTheDocument();
  expect(screen.getByText(corrupt.retained)).toBeInTheDocument();
  expect(screen.getByText(corrupt.recovery)).toBeInTheDocument();
  expect(screen.getByText("database_corrupt")).toBeInTheDocument();
});

test("offers only the recoveries the error allows", () => {
  show({ error: inUse, snapshots: [] });

  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Restore a snapshot" })).not.toBeInTheDocument();
});

test("offers no recovery buttons when the app cannot perform one", () => {
  show({ error: { ...inUse, actions: [] }, snapshots: [] });

  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

test("try again runs from the keyboard", async () => {
  const user = userEvent.setup();
  const props = show({ error: inUse, snapshots: [] });

  await user.tab();
  expect(screen.getByRole("button", { name: "Try again" })).toHaveFocus();
  await user.keyboard("{Enter}");

  expect(props.onRetry).toHaveBeenCalledTimes(1);
});

test("restoring needs a second, explicit confirmation that explains the effect", async () => {
  const user = userEvent.setup();
  const props = show();

  await user.click(restoreTrigger());

  expect(props.onRestore).not.toHaveBeenCalled();
  const confirm = screen.getByRole("group", {
    name: "Replace the library’s database with this snapshot?",
  });
  expect(confirm).toHaveTextContent("moved aside and kept, not deleted");
  expect(confirm).toHaveTextContent("Audio files are not changed");

  await user.click(within(confirm).getByRole("button", { name: "Restore this snapshot" }));

  expect(props.onRestore).toHaveBeenCalledExactlyOnceWith(snapshot.name);
  expect(confirmation()).not.toBeInTheDocument();
});

test("the confirmation opens on Cancel, and cancelling returns focus without restoring", async () => {
  const user = userEvent.setup();
  const props = show();

  restoreTrigger().focus();
  await user.keyboard("{Enter}");
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  expect(restoreTrigger()).toHaveAttribute("aria-expanded", "true");

  await user.keyboard("{Enter}");

  expect(props.onRestore).not.toHaveBeenCalled();
  expect(confirmation()).not.toBeInTheDocument();
  expect(restoreTrigger()).toHaveFocus();
});

test("says so when there is no snapshot to restore from", () => {
  show({ snapshots: [] });

  expect(screen.getByText(/No snapshots were found/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^Restore/ })).not.toBeInTheDocument();
});

test("while a recovery runs it reports progress and ignores further requests", async () => {
  const user = userEvent.setup();
  const props = show({ pending: "restore" });

  expect(screen.getByRole("status")).toHaveTextContent("Restoring the snapshot…");
  const retry = screen.getByRole("button", { name: "Try again" });
  expect(retry).toHaveAttribute("aria-disabled", "true");

  await user.click(retry);
  await user.click(restoreTrigger());

  expect(props.onRetry).not.toHaveBeenCalled();
  expect(confirmation()).not.toBeInTheDocument();
});

test("a failed recovery is announced without replacing the original failure", () => {
  show({ recoveryError: snapshotGone });

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent(snapshotGone.message);
  expect(alert).toHaveTextContent(snapshotGone.recovery);
  expect(screen.getByText(corrupt.message)).toBeInTheDocument();
  expect(restoreTrigger()).toBeInTheDocument();
});
