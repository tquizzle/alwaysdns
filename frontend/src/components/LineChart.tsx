import { useMemo, useState, type PointerEvent } from 'react';
import type { TimelinePoint } from '../api/types';
import { formatCompact, formatDateTime, formatInt, formatTime, parseTimestamp } from '../lib/format';
import { useAccent } from '../lib/prefs';

const W = 640;
const H = 240;
const PAD = { left: 44, right: 12, top: 14, bottom: 28 };
const INNER_W = W - PAD.left - PAD.right;
const INNER_H = H - PAD.top - PAD.bottom;

const COLOR_BLOCKED = '#f87171';

/** Rounds up to a 1, 2, 2.5, 5 or 10 multiple so gridlines read cleanly. */
function niceCeil(value: number): number {
  if (value <= 0) return 1;
  const exp = Math.floor(Math.log10(value));
  const base = 10 ** exp;
  for (const step of [1, 2, 2.5, 5, 10]) {
    if (step * base >= value) return step * base;
  }
  return 10 * base;
}

interface Props {
  points: TimelinePoint[];
  /** Formats the x-axis labels. Defaults to local time of day. */
  formatTick?: (t: string | number) => string;
  /** The last point is a bucket still filling. It is drawn dashed so a partial count does not read as a drop. */
  openLast?: boolean;
}

/** Two-series SVG line chart: total queries (cyan) and blocked queries (red). */
export function LineChart({ points, formatTick = formatTime, openLast = false }: Props) {
  const [hover, setHover] = useState<number | null>(null);
  const accent = useAccent();

  const data = useMemo(
    () =>
      points.map((p) => ({
        t: p.t,
        total: Number.isFinite(p.total) ? p.total : 0,
        blocked: Number.isFinite(p.blocked) ? p.blocked : 0,
      })),
    [points],
  );

  const yMax = useMemo(() => niceCeil(Math.max(1, ...data.map((d) => Math.max(d.total, d.blocked)))), [data]);
  const n = data.length;

  const xAt = (i: number) => (n <= 1 ? PAD.left + INNER_W / 2 : PAD.left + (i / (n - 1)) * INNER_W);
  const yAt = (v: number) => PAD.top + INNER_H - (v / yMax) * INNER_H;

  const hasOpen = openLast && n > 2;
  const closedEnd = hasOpen ? n - 2 : n - 1;
  const pathFor = (key: 'total' | 'blocked', from: number, to: number): string =>
    data
      .slice(from, to + 1)
      .map((d, k) => `${k === 0 ? 'M' : 'L'}${xAt(from + k).toFixed(1)},${yAt(d[key]).toFixed(1)}`)
      .join(' ');

  const totalPath = pathFor('total', 0, closedEnd);
  const blockedPath = pathFor('blocked', 0, closedEnd);
  const openTotalPath = hasOpen ? pathFor('total', n - 2, n - 1) : '';
  const openBlockedPath = hasOpen ? pathFor('blocked', n - 2, n - 1) : '';
  const baseY = (PAD.top + INNER_H).toFixed(1);
  const areaPath =
    closedEnd > 0 ? `${totalPath} L${xAt(closedEnd).toFixed(1)},${baseY} L${xAt(0).toFixed(1)},${baseY} Z` : '';

  const gridValues = [0, 0.25, 0.5, 0.75, 1].map((f) => f * yMax);

  const handleMove = (e: PointerEvent<SVGSVGElement>) => {
    if (n === 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const svgX = ((e.clientX - rect.left) / rect.width) * W;
    const frac = n <= 1 ? 0 : (svgX - PAD.left) / INNER_W;
    const idx = Math.round(Math.min(1, Math.max(0, frac)) * (n - 1));
    setHover(idx);
  };

  const hovered = hover !== null && data[hover] ? data[hover] : null;
  const hoverX = hover !== null ? xAt(hover) : 0;
  const first = n > 0 ? data[0].t : null;
  const last = n > 0 ? data[n - 1].t : null;
  const mid = n > 2 ? data[Math.floor((n - 1) / 2)].t : null;

  const summary = `Queries and blocks over time, ${n} points. Peak ${formatInt(yMax)} per interval.`;

  return (
    <div className="line-chart">
      <div className="legend" aria-hidden="true">
        <span><i style={{ background: accent }} /> Queries</span>
        <span><i style={{ background: COLOR_BLOCKED }} /> Blocked</span>
      </div>

      <div className="chart-frame">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          role="img"
          aria-label={summary}
          onPointerMove={handleMove}
          onPointerLeave={() => setHover(null)}
        >
          <defs>
            <linearGradient id="area-total" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={accent} stopOpacity="0.35" />
              <stop offset="100%" stopColor={accent} stopOpacity="0" />
            </linearGradient>
          </defs>

          {gridValues.map((v) => (
            <line
              key={v}
              x1={PAD.left}
              x2={W - PAD.right}
              y1={yAt(v)}
              y2={yAt(v)}
              style={{ stroke: 'rgba(var(--edge-rgb), 0.14)' }}
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
            />
          ))}

          {areaPath && <path d={areaPath} fill="url(#area-total)" />}
          {n > 0 && (
            <>
              <path d={totalPath} fill="none" stroke={accent} strokeWidth={2} vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
              <path d={blockedPath} fill="none" stroke={COLOR_BLOCKED} strokeWidth={2} vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
            </>
          )}
          {hasOpen && (
            <>
              <path d={openTotalPath} fill="none" stroke={accent} strokeWidth={2} strokeDasharray="4 4" strokeOpacity={0.7} vectorEffect="non-scaling-stroke" />
              <path d={openBlockedPath} fill="none" stroke={COLOR_BLOCKED} strokeWidth={2} strokeDasharray="4 4" strokeOpacity={0.7} vectorEffect="non-scaling-stroke" />
            </>
          )}

          {hovered && (
            <g>
              <line x1={hoverX} x2={hoverX} y1={PAD.top} y2={PAD.top + INNER_H} stroke="rgba(230,244,255,0.35)" strokeDasharray="3 3" vectorEffect="non-scaling-stroke" />
              <circle cx={hoverX} cy={yAt(hovered.total)} r={3.5} fill={accent} />
              <circle cx={hoverX} cy={yAt(hovered.blocked)} r={3.5} fill={COLOR_BLOCKED} />
            </g>
          )}
        </svg>

        <div className="axis-y" aria-hidden="true">
          {[...gridValues].reverse().map((v) => (
            <span key={v} style={{ top: `${((yAt(v)) / H) * 100}%` }}>{formatCompact(v)}</span>
          ))}
        </div>

        {hovered && (
          <div
            className="chart-tooltip"
            style={{ left: `calc(${PAD.left}px + (100% - ${PAD.left}px) * ${hoverX / W})` }}
            role="status"
          >
            <strong>{formatDateTime(hovered.t)}</strong>
            <span style={{ color: accent }}>Queries {formatInt(hovered.total)}</span>
            <span style={{ color: COLOR_BLOCKED }}>Blocked {formatInt(hovered.blocked)}</span>
            {hasOpen && hover === n - 1 && <span className="muted">Still counting</span>}
          </div>
        )}
      </div>

      <div className="axis-x" aria-hidden="true">
        <span>{first !== null ? formatTick(first) : ''}</span>
        <span>{mid !== null ? formatTick(mid) : ''}</span>
        <span>{last !== null && parseTimestamp(last) ? formatTick(last) : ''}</span>
      </div>
    </div>
  );
}
