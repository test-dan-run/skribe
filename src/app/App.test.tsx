import { render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";

import { call } from "../ipc";
import { corrupt, library, ready, unavailable } from "../test/fixtures";
import { App } from "./App";

vi.mock("../ipc", () => ({ call: vi.fn() }));
const callMock = vi.mocked(call);

test("says the library is opening, then shows an honest empty state", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: ready });

  render(<App />);

  expect(screen.getByRole("status")).toHaveTextContent("Opening your library…");
  expect(await screen.findByRole("heading", { name: "Meetings" })).toBeInTheDocument();
  expect(screen.getByText("No meetings yet")).toBeInTheDocument();
  expect(screen.getByText(/are not available in it yet/)).toBeInTheDocument();
  // Nothing on screen offers a feature that does not exist yet.
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(screen.getByText(library.locations.library)).toBeInTheDocument();
});

test("reports what opening the library had to do", async () => {
  const message = "The library was updated for this version of Skribe.";
  callMock.mockResolvedValueOnce({
    ok: true,
    value: {
      state: "ready",
      library: { ...library, openNotices: [{ kind: "library_updated", message }] },
    },
  });

  render(<App />);

  expect(await screen.findByText(message)).toBeInTheDocument();
});

test("shows the recovery screen instead of the app when the library cannot be opened", async () => {
  callMock.mockResolvedValueOnce({ ok: true, value: unavailable(corrupt) });

  render(<App />);

  expect(
    await screen.findByRole("heading", { name: "Skribe could not open your library" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Meetings" })).not.toBeInTheDocument();
});
