const timestampFormat = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

/** A Unix millisecond timestamp in the user's locale and time zone. */
export function formatTimestamp(unixMs: number): string {
  return timestampFormat.format(new Date(unixMs));
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${String(bytes)} bytes`;
  }
  const units = ["kB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit] ?? "GB"}`;
}
