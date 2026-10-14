/**
 * Conversions between a planned time as Unix milliseconds and the value of a
 * `datetime-local` input, which is a wall-clock time in the user's own time zone.
 */

const two = (value: number) => String(value).padStart(2, "0");

export function toInputValue(unixMs: number | null): string {
  if (unixMs === null) {
    return "";
  }
  const date = new Date(unixMs);
  const year = String(date.getFullYear()).padStart(4, "0");
  return `${year}-${two(date.getMonth() + 1)}-${two(date.getDate())}T${two(date.getHours())}:${two(date.getMinutes())}`;
}

/** `null` for an empty input, `"invalid"` for text that is not a date and time. */
export function fromInputValue(value: string): number | null | "invalid" {
  if (value === "") {
    return null;
  }
  const unixMs = new Date(value).getTime();
  return Number.isFinite(unixMs) && unixMs >= 0 ? unixMs : "invalid";
}
