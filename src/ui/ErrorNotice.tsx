import type { ReactNode } from "react";

import type { CommandError } from "../ipc";

/** Technical detail for a failure, collapsed by default. */
export function ErrorDetails({ error }: { error: CommandError }) {
  return (
    <details className="details">
      <summary>Technical details</summary>
      <dl className="facts facts--compact">
        <dt>Code</dt>
        <dd>
          <code>{error.code}</code>
        </dd>
        {error.detail !== null && (
          <>
            <dt>Detail</dt>
            <dd>
              <code className="wrap">{error.detail}</code>
            </dd>
          </>
        )}
      </dl>
    </details>
  );
}

/**
 * A failed action, announced when it appears: what happened, what was kept and what can be
 * done. `children` are the recoveries offered for it.
 */
export function ErrorNotice({ error, children }: { error: CommandError; children?: ReactNode }) {
  return (
    <div className="notice notice--error" role="alert">
      <p>
        <strong>{error.message}</strong>
      </p>
      <p>{error.retained}</p>
      <p>{error.recovery}</p>
      {children !== undefined && <div className="notice__actions">{children}</div>}
      <ErrorDetails error={error} />
    </div>
  );
}
