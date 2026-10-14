import { useEffect, useId, useRef, useState } from "react";
import type { ReactNode } from "react";

import { Button } from "./Button";

/**
 * An inline confirmation: a trigger button and a panel that asks the question. The panel
 * opens with focus on Cancel, the safe choice, and closing it returns focus to the trigger.
 * It is a disclosure, not a modal dialog, so the rest of the page stays reachable.
 */
export interface Confirmation {
  open: boolean;
  ask: () => void;
  close: () => void;
  panelId: string;
  questionId: string;
}

export function useConfirmation(): Confirmation {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const questionId = useId();
  return {
    open,
    ask: () => {
      setOpen(true);
    },
    close: () => {
      setOpen(false);
    },
    panelId,
    questionId,
  };
}

interface ConfirmTriggerProps {
  confirmation: Confirmation;
  /** The accessible name, when the visible text alone does not say what is acted on. */
  label?: string;
  unavailable?: boolean;
  variant?: "default" | "danger";
  children: ReactNode;
}

export function ConfirmTrigger({
  confirmation,
  label,
  unavailable = false,
  variant = "default",
  children,
}: ConfirmTriggerProps) {
  const button = useRef<HTMLButtonElement>(null);
  const wasOpen = useRef(false);
  const { open } = confirmation;

  // Closing the panel, either way, hands focus back to the button that opened it.
  useEffect(() => {
    if (wasOpen.current && !open) {
      button.current?.focus();
    }
    wasOpen.current = open;
  }, [open]);

  return (
    <Button
      ref={button}
      variant={variant}
      aria-label={label}
      aria-expanded={open}
      aria-controls={confirmation.panelId}
      unavailable={unavailable}
      onClick={() => {
        if (!open) {
          confirmation.ask();
        }
      }}
    >
      {children}
    </Button>
  );
}

interface ConfirmPanelProps {
  confirmation: Confirmation;
  question: string;
  confirmLabel: string;
  variant?: "primary" | "danger";
  onConfirm: () => void;
  /** What confirming does and what is kept. */
  children: ReactNode;
}

export function ConfirmPanel(props: ConfirmPanelProps) {
  return <div id={props.confirmation.panelId}>{props.confirmation.open && <Question {...props} />}</div>;
}

function Question({
  confirmation,
  question,
  confirmLabel,
  variant = "primary",
  onConfirm,
  children,
}: ConfirmPanelProps) {
  const cancel = useRef<HTMLButtonElement>(null);

  // Opening the confirmation moves focus to the safe choice.
  useEffect(() => {
    cancel.current?.focus();
  }, []);

  return (
    <div
      className="confirm"
      role="group"
      aria-labelledby={confirmation.questionId}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          confirmation.close();
        }
      }}
    >
      <p id={confirmation.questionId}>
        <strong>{question}</strong>
      </p>
      {children}
      <div className="confirm__actions">
        <Button ref={cancel} onClick={confirmation.close}>
          Cancel
        </Button>
        <Button
          variant={variant}
          onClick={() => {
            confirmation.close();
            onConfirm();
          }}
        >
          {confirmLabel}
        </Button>
      </div>
    </div>
  );
}
