import type { CSSProperties } from 'react';

export type Tone = 'cyan' | 'red' | 'green' | 'yellow';

interface Props {
  label: string;
  value: string;
  unit?: string;
  /** Preformatted percentage or other secondary figure. */
  pct?: string;
  tone: Tone;
}

/** Stat card with a coloured top bar and corner brackets. */
export function KpiCard({ label, value, unit, pct, tone }: Props) {
  return (
    <div className={`kpi tone-${tone}`}>
      <span className="kpi-bar" aria-hidden="true" />
      <span className="kpi-corner tl" aria-hidden="true" />
      <span className="kpi-corner tr" aria-hidden="true" />
      <span className="kpi-corner bl" aria-hidden="true" />
      <span className="kpi-corner br" aria-hidden="true" />
      <div className="kpi-label">{label}</div>
      <div className="kpi-value">
        <span className="kpi-number" style={{ '--chars': value.length } as CSSProperties}>
          {value}
        </span>
        {unit && <span className="kpi-unit">{unit}</span>}
      </div>
      <div className="kpi-pct">{pct ?? ' '}</div>
    </div>
  );
}
