import type { NameCount } from '../api/types';
import { formatInt } from '../lib/format';

interface Props {
  items: NameCount[];
  tone: 'cyan' | 'red';
  emptyText: string;
  /** Show this many rows and scroll for the rest. Omit to show every row. */
  maxRows?: number;
}

// Row geometry, matching the bar-list CSS: meta line and bar, then the gap.
const ROW_HEIGHT = 28;
const ROW_GAP = 10;
const REASON_HEIGHT = 18;

/** Height of `rows` rows, so the scrolling box shows exactly that many. */
function heightFor(rows: number, withReasons: boolean): number {
  const base = rows * ROW_HEIGHT + (rows - 1) * ROW_GAP;
  return withReasons ? base + rows * REASON_HEIGHT : base;
}

/** Horizontal bars for ranked names, scaled to the largest count. */
export function BarList({ items, tone, emptyText, maxRows }: Props) {
  if (items.length === 0) {
    return <div className="state state-empty">{emptyText}</div>;
  }
  const max = Math.max(1, ...items.map((i) => i.count));
  const list = (
    <ol className="bar-list">
      {items.map((item) => (
        <li key={item.name} className={`bar-row tone-${tone}`}>
          <div className="bar-meta">
            <span className="bar-name" title={item.name}>{item.name}</span>
            <span className="bar-count">{formatInt(item.count)}</span>
          </div>
          {item.reason && (
            <div className="bar-reason" title={`Blocked by ${item.reason}`}>
              Blocked by {item.reason}
            </div>
          )}
          <div className="bar-track">
            <div className="bar-fill" style={{ width: `${(item.count / max) * 100}%` }} />
          </div>
        </li>
      ))}
    </ol>
  );

  if (maxRows === undefined || items.length <= maxRows) return list;

  const withReasons = items.some((i) => i.reason);
  return (
    <div
      className="bar-scroll"
      style={{ maxHeight: heightFor(maxRows, withReasons) }}
      tabIndex={0}
      aria-label={`${items.length} rows, scroll to see all`}
    >
      {list}
    </div>
  );
}
