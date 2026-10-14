import type { ButtonHTMLAttributes, MouseEvent, Ref } from "react";

interface ButtonProps extends Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "type" | "disabled" | "onClick"
> {
  variant?: "default" | "primary" | "danger";
  type?: "button" | "submit";
  /**
   * Shown but not usable right now. The button stays focusable, so a keyboard user does not
   * lose their place while something is in progress.
   */
  unavailable?: boolean;
  onClick?: () => void;
  ref?: Ref<HTMLButtonElement>;
}

export function Button({
  variant = "default",
  type = "button",
  unavailable = false,
  onClick,
  className,
  ref,
  ...rest
}: ButtonProps) {
  const classes = ["button", variant === "default" ? "" : `button--${variant}`, className ?? ""]
    .filter((name) => name !== "")
    .join(" ");
  return (
    <button
      {...rest}
      ref={ref}
      // The set of types is closed, so this is not a dynamic type.
      type={type === "submit" ? "submit" : "button"}
      className={classes}
      aria-disabled={unavailable}
      onClick={(event: MouseEvent<HTMLButtonElement>) => {
        if (unavailable) {
          // Also stops a submit button from submitting its form.
          event.preventDefault();
          return;
        }
        onClick?.();
      }}
    />
  );
}
