import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ConfirmPanel, ConfirmTrigger, useConfirmation } from "./Confirmation";

function Example({ onConfirm, unavailable = false }: { onConfirm: () => void; unavailable?: boolean }) {
  const confirmation = useConfirmation();
  return (
    <>
      <ConfirmTrigger confirmation={confirmation} label="Remove the item" unavailable={unavailable}>
        Remove…
      </ConfirmTrigger>
      <ConfirmPanel
        confirmation={confirmation}
        question="Remove it?"
        confirmLabel="Remove"
        onConfirm={onConfirm}
      >
        <p>It is taken out of the list.</p>
      </ConfirmPanel>
    </>
  );
}

const trigger = () => screen.getByRole("button", { name: "Remove the item" });

test("opens on Cancel and does nothing until confirmed", async () => {
  const user = userEvent.setup();
  const onConfirm = vi.fn();
  render(<Example onConfirm={onConfirm} />);

  trigger().focus();
  await user.keyboard("{Enter}");

  const question = screen.getByRole("group", { name: "Remove it?" });
  expect(question).toHaveTextContent("It is taken out of the list.");
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  expect(trigger()).toHaveAttribute("aria-expanded", "true");
  expect(onConfirm).not.toHaveBeenCalled();
});

test("cancelling, by button or Escape, returns focus to the trigger", async () => {
  const user = userEvent.setup();
  const onConfirm = vi.fn();
  render(<Example onConfirm={onConfirm} />);

  await user.click(trigger());
  await user.keyboard("{Enter}");
  expect(screen.queryByRole("group", { name: "Remove it?" })).not.toBeInTheDocument();
  expect(trigger()).toHaveFocus();

  await user.keyboard("{Enter}");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("group", { name: "Remove it?" })).not.toBeInTheDocument();
  expect(trigger()).toHaveFocus();
  expect(onConfirm).not.toHaveBeenCalled();
});

test("confirming runs the action once and closes", async () => {
  const user = userEvent.setup();
  const onConfirm = vi.fn();
  render(<Example onConfirm={onConfirm} />);

  await user.click(trigger());
  await user.click(screen.getByRole("button", { name: "Remove" }));

  expect(onConfirm).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("group", { name: "Remove it?" })).not.toBeInTheDocument();
});

test("an unavailable trigger stays focusable but does not open", async () => {
  const user = userEvent.setup();
  render(<Example onConfirm={vi.fn()} unavailable />);

  await user.tab();
  expect(trigger()).toHaveFocus();
  await user.keyboard("{Enter}");

  expect(trigger()).toHaveAttribute("aria-disabled", "true");
  expect(screen.queryByRole("group", { name: "Remove it?" })).not.toBeInTheDocument();
});
