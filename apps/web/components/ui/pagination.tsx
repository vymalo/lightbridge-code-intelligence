/** Range label + prev/page/next control on a daisyUI `join` (ADR-0027). Controlled: the caller owns
 * the page index (URL state via nuqs). `onPageChange(null)` clears the param on the way to page 0 —
 * callers pass `prev || null` so the first page yields a clean URL. */
export function Pagination({
  current,
  pageCount,
  rangeLabel,
  onPageChange,
  className,
}: {
  current: number;
  pageCount: number;
  rangeLabel: string;
  onPageChange: (page: number | null) => void;
  className?: string;
}) {
  return (
    <div className={className}>
      <span>{rangeLabel}</span>
      <div className="join">
        <button
          type="button"
          className="btn btn-xs join-item"
          disabled={current <= 0}
          onClick={() => onPageChange(current - 1 || null)}
        >
          Prev
        </button>
        <span className="btn btn-xs join-item pointer-events-none tabular-nums">
          {current + 1} / {pageCount}
        </span>
        <button
          type="button"
          className="btn btn-xs join-item"
          disabled={current >= pageCount - 1}
          onClick={() => onPageChange(current + 1)}
        >
          Next
        </button>
      </div>
    </div>
  );
}
