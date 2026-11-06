import { useId, type ReactNode } from 'react';
import { clamp01 } from '../lib/format';

interface GaugeProps {
  /** Fill fraction, 0 to 1. */
  value: number;
  size?: number;
  stroke?: number;
  color: string;
  /** When set, the ring fades from `color` to this colour. */
  colorEnd?: string;
  trackColor?: string;
  /** Adds the tick ring and the orb backdrop used by the hero gauge. */
  orb?: boolean;
  label: string;
  children?: ReactNode;
}

/** Hand-drawn SVG ring gauge with an optional glowing orb backdrop. */
export function Gauge({
  value,
  size = 200,
  stroke = 12,
  color,
  colorEnd,
  trackColor = 'rgba(var(--edge-rgb), 0.12)',
  orb = false,
  label,
  children,
}: GaugeProps) {
  const id = useId().replace(/:/g, '');
  const radius = size / 2 - stroke - (orb ? 10 : 0);
  const circumference = 2 * Math.PI * radius;
  const fraction = clamp01(value);
  const center = size / 2;

  return (
    <div className="gauge" style={{ width: size, height: size }}>
      <svg
        viewBox={`0 0 ${size} ${size}`}
        width={size}
        height={size}
        role="img"
        aria-label={label}
      >
        <defs>
          <radialGradient id={`${id}-orb`} cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor={color} stopOpacity="0.22" />
            <stop offset="70%" style={{ stopColor: 'var(--bg-bottom)', stopOpacity: 0.6 }} />
            <stop offset="100%" style={{ stopColor: 'var(--bg-top)', stopOpacity: 0 }} />
          </radialGradient>
          {colorEnd && (
            <linearGradient id={`${id}-ring`} x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stopColor={color} />
              <stop offset="100%" stopColor={colorEnd} />
            </linearGradient>
          )}
          <filter id={`${id}-glow`} x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="4" result="blur" />
            <feMerge>
              <feMergeNode in="blur" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
        </defs>

        {orb && <circle cx={center} cy={center} r={size / 2 - 2} fill={`url(#${id}-orb)`} />}

        {orb &&
          Array.from({ length: 60 }, (_, i) => {
            const angle = (i / 60) * 2 * Math.PI;
            const outer = size / 2 - 2;
            const inner = outer - (i % 5 === 0 ? 8 : 4);
            return (
              <line
                key={i}
                x1={center + Math.cos(angle) * inner}
                y1={center + Math.sin(angle) * inner}
                x2={center + Math.cos(angle) * outer}
                y2={center + Math.sin(angle) * outer}
                stroke={color}
                strokeOpacity={i % 5 === 0 ? 0.7 : 0.3}
                strokeWidth={1}
              />
            );
          })}

        <circle cx={center} cy={center} r={radius} fill="none" style={{ stroke: trackColor }} strokeWidth={stroke} />
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke={colorEnd ? `url(#${id}-ring)` : color}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${circumference * fraction} ${circumference}`}
          transform={`rotate(-90 ${center} ${center})`}
          filter={`url(#${id}-glow)`}
        />
      </svg>
      {children && <div className="gauge-center">{children}</div>}
    </div>
  );
}
