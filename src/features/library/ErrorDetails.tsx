import type { CommandError } from "../../ipc";

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
