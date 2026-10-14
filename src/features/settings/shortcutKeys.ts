/** The parts of a keyboard event that decide a shortcut. */
export interface KeyPress {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/**
 * The shortcut a key press spells, such as `Ctrl+Shift+F9`, or `null` when the press is not a
 * combination: no Ctrl, Alt or Windows key is held, or the key is not one a shortcut can use.
 * Rust decides whether the result is acceptable; this only saves typing it.
 */
export function shortcutFromKeyPress(press: KeyPress): string | null {
  if (!(press.ctrlKey || press.altKey || press.metaKey)) {
    return null;
  }
  const key =
    /^Key([A-Z])$/.exec(press.code)?.[1] ??
    /^Digit([0-9])$/.exec(press.code)?.[1] ??
    /^(F(?:[1-9]|1[0-9]|2[0-4]))$/.exec(press.code)?.[1];
  if (key === undefined) {
    return null;
  }
  const parts = [
    press.ctrlKey ? "Ctrl" : "",
    press.altKey ? "Alt" : "",
    press.shiftKey ? "Shift" : "",
    press.metaKey ? "Super" : "",
    key,
  ];
  return parts.filter((part) => part !== "").join("+");
}
