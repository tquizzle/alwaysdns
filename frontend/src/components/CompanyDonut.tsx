import type { NameCount } from '../api/types';
import { formatInt, formatPercent } from '../lib/format';

/** Fixed colours so each company keeps its colour across refreshes. */
export const COMPANY_COLORS: Record<string, string> = {
  Apple: '#e2e8f0',
  Google: '#60a5fa',
  Microsoft: '#34d399',
  Amazon: '#fbbf24',
  Meta: '#a78bfa',
};
const OTHERS_COLOR = '#475569';

const RADIUS = 70;
const STROKE = 26;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

interface Slice {
  name: string;
  count: number;
  share: number;
  color: string;
}

interface Props {
  /** Query counts per company, from the server. */
  companies: NameCount[];
  /** All queries in the range, so each company's share is of the whole. */
  total: number;
}

/**
 * GAFAM dominance: each company's share of all queries, and the rest as "Others".
 * Drawn with stacked stroke-dash segments, so it needs no charting library.
 */
export function CompanyDonut({ companies, total }: Props) {
  const slices: Slice[] = companies
    .filter((c) => c.count > 0)
    .map((c) => ({
      name: c.name,
      count: c.count,
      share: total > 0 ? c.count / total : 0,
      color: COMPANY_COLORS[c.name] ?? OTHERS_COLOR,
    }));
  const named = slices.reduce((sum, s) => sum + s.count, 0);
  const others = Math.max(0, total - named);
  if (others > 0) {
    slices.push({
      name: 'Others',
      count: others,
      share: total > 0 ? others / total : 0,
      color: OTHERS_COLOR,
    });
  }

  if (total === 0) {
    return <div className="state state-empty">No queries in this range yet.</div>;
  }

  let offset = 0;
  return (
    <div className="donut-wrap">
      <svg
        className="donut"
        viewBox="0 0 200 200"
        role="img"
        aria-label={`Company share of queries: ${slices
          .map((s) => `${s.name} ${formatPercent(s.share, 2)}`)
          .join(', ')}`}
      >
        <g transform="rotate(-90 100 100)">
          {slices.map((s) => {
            const length = s.share * CIRCUMFERENCE;
            const segment = (
              <circle
                key={s.name}
                cx="100"
                cy="100"
                r={RADIUS}
                fill="none"
                stroke={s.color}
                strokeWidth={STROKE}
                strokeDasharray={`${length} ${CIRCUMFERENCE - length}`}
                strokeDashoffset={-offset}
              >
                <title>{`${s.name}: ${formatInt(s.count)} queries`}</title>
              </circle>
            );
            offset += length;
            return segment;
          })}
        </g>
      </svg>
      <ul className="donut-legend">
        {slices.map((s) => (
          <li key={s.name}>
            <span className="legend-swatch" style={{ background: s.color }} aria-hidden="true" />
            <span className="legend-name">{s.name}</span>
            <span className="legend-share">{formatPercent(s.share, 2)}</span>
            <span className="legend-count muted">{formatInt(s.count)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
