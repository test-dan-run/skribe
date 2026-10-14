const two = (value: number) => String(value).padStart(2, "0");

/** Microseconds as `m:ss`, or `h:mm:ss` from an hour up. Fractions of a second are dropped. */
export function formatDuration(micros: number): string {
  const total = Math.max(0, Math.floor(micros / 1_000_000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  return hours > 0
    ? `${String(hours)}:${two(minutes)}:${two(seconds)}`
    : `${String(minutes)}:${two(seconds)}`;
}

/** Seconds of available recording time in words, rounded down, such as "about 36 hours". */
export function formatRoom(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  if (hours >= 2) {
    return `about ${String(hours)} hours`;
  }
  const minutes = Math.floor(seconds / 60);
  return minutes === 1 ? "about 1 minute" : `about ${String(minutes)} minutes`;
}
