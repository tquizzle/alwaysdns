import { useState } from 'react';
import { useQueryLog, useStats } from '../api/queries';
import type { RangeKey, Stats, UpstreamStatus } from '../api/types';
import { BarList } from '../components/BarList';
import { CompanyDonut } from '../components/CompanyDonut';
import { Gauge } from '../components/Gauge';
import { TrafficMap } from '../components/TrafficMap';
import { KpiCard, type Tone } from '../components/KpiCard';
import { LineChart } from '../components/LineChart';
import { Panel } from '../components/Panel';
import { QueryLogTable } from '../components/QueryLogTable';
import { SegmentedControl } from '../components/SegmentedControl';
import { HistoryPanel } from '../components/HistoryPanel';
import { EmptyState, ErrorState, LoadingState, Notice } from '../components/States';
import {
  clamp01,
  formatInt,
  formatPercent,
  formatTime,
  ratioOf,
  splitMs,
} from '../lib/format';
import { usePrefs } from '../lib/prefs';
import { PROTOCOL_LABEL } from '../lib/resolvers';

const RANGE_OPTIONS: { value: RangeKey; label: string }[] = [
  { value: 'hour', label: 'Hour' },
  { value: 'day', label: 'Day' },
  { value: 'week', label: 'Week' },
  { value: 'month', label: 'Month' },
  { value: 'year', label: 'Year' },
];

const RANGE_LABEL: Record<RangeKey, string> = {
  hour: 'last hour',
  day: 'last 24 hours',
  week: 'last 7 days',
  month: 'last 30 days',
  year: 'last 12 months',
};

const COLOR = {
  cyan: '#22d3ee',
  yellow: '#facc15',
  red: '#f87171',
  green: '#4ade80',
};

const NO_TRAFFIC_HINT = "No domains yet. Point a device's DNS at this server to see traffic.";

function latencyTone(ms: number): string {
  if (!Number.isFinite(ms)) return COLOR.yellow;
  if (ms < 100) return COLOR.green;
  if (ms < 250) return COLOR.yellow;
  return COLOR.red;
}

/** Link target that opens the query log filtered to one domain. */
function queryLogLink(domain: string): string {
  return `#/querylog?domain=${encodeURIComponent(domain)}`;
}

export function Dashboard() {
  const [range, setRange] = useState<RangeKey>('hour');
  const stats = useStats(range);
  const data = stats.data;

  return (
    <div className="dashboard">
      <div className="page-bar">
        <div>
          <h1>Network overview</h1>
          <p className="muted small">Live, refreshed every 5 seconds · {RANGE_LABEL[range]}</p>
        </div>
        <SegmentedControl label="Time range" options={RANGE_OPTIONS} value={range} onChange={setRange} />
      </div>

      {data && stats.isError && (
        <Notice tone="error">
          Could not refresh. Showing the last update from {formatTime(stats.dataUpdatedAt)}.
        </Notice>
      )}
      {!data && stats.isPending && <LoadingState label="Loading statistics" />}
      {!data && stats.isError && <ErrorState error={stats.error} onRetry={() => stats.refetch()} />}
      {data && <DashboardBody stats={data} />}

      <HistoryPanel />
    </div>
  );
}

/** Which resolvers are healthy, as one line a glance can read. */
function healthSummary(upstreams: UpstreamStatus[], avgMs: number): { text: string; degraded: boolean } {
  if (upstreams.length === 0) return { text: 'No resolvers reported.', degraded: true };
  const up = upstreams.filter((u) => u.ok).length;
  const latency = splitMs(avgMs);
  const average = Number.isFinite(avgMs) ? ` · average ${latency.value} ${latency.unit}` : '';
  return { text: `${up} of ${upstreams.length} resolvers up${average}`, degraded: up < upstreams.length };
}

function DashboardBody({ stats }: { stats: Stats }) {
  const blockRate = Number.isFinite(stats.block_rate)
    ? stats.block_rate
    : ratioOf(stats.blocked, stats.total) ?? 0;
  const latency = splitMs(stats.avg_ms);
  const upstreamsOk = stats.upstreams.filter((u) => u.ok).length;
  const health = healthSummary(stats.upstreams, stats.avg_ms);

  const kpis: { label: string; value: string; unit?: string; pct?: string; tone: Tone }[] = [
    { label: 'Queries', value: formatInt(stats.total), tone: 'cyan' },
    {
      label: 'Blocked',
      value: formatInt(stats.blocked),
      pct: `${formatPercent(ratioOf(stats.blocked, stats.total))} of total`,
      tone: 'red',
    },
    {
      label: 'Cached',
      value: formatInt(stats.cached),
      pct: `${formatPercent(ratioOf(stats.cached, stats.total))} of total`,
      tone: 'cyan',
    },
    {
      label: 'Forwarded',
      value: formatInt(stats.forwarded),
      pct: `${formatPercent(ratioOf(stats.forwarded, stats.total))} of total`,
      tone: 'green',
    },
    {
      label: 'Avg latency',
      value: latency.value,
      unit: latency.unit,
      pct: stats.upstreams.length > 0 ? `${upstreamsOk}/${stats.upstreams.length} upstreams ok` : undefined,
      tone: Number.isFinite(stats.avg_ms) && stats.avg_ms >= 250 ? 'yellow' : 'green',
    },
    {
      label: 'Active clients',
      value: formatInt(stats.active_clients),
      tone: 'cyan',
    },
  ];

  return (
    // One shared two-column grid: every panel sits on the same column lines.
    // Rows that need the full width carry the dash-span class.
    <div className="dash-grid">
      <section className="panel dash-span" aria-label="Total queries, key figures and block rate">
        <div className="dash-top-grid">
          <div className="orb-slot orb-total">
            <Gauge
              value={1 - blockRate}
              size={240}
              stroke={12}
              color={COLOR.green}
              colorEnd={COLOR.cyan}
              orb
              label={`Total queries ${formatInt(stats.total)}, ${formatPercent(1 - blockRate)} answered`}
            >
              <div className="hero-total">
                <span className="hero-label">Total queries</span>
                <span className="hero-number">{formatInt(stats.total)}</span>
                <span className="hero-label">{formatPercent(1 - blockRate)} answered</span>
              </div>
            </Gauge>
          </div>
          <aside className="dash-kpis" aria-label="Key figures">
            {kpis.map((k) => (
              <KpiCard key={k.label} {...k} />
            ))}
          </aside>
          <div className="orb-slot orb-block">
            <Gauge value={blockRate} size={240} stroke={12} color={COLOR.red} orb label={`Block rate ${formatPercent(blockRate)}`}>
              <div className="hero-total">
                <span className="hero-label">Block rate</span>
                <span className="hero-number is-red">{formatPercent(blockRate)}</span>
                <span className="hero-label">{formatInt(stats.blocked)} of {formatInt(stats.total)}</span>
              </div>
            </Gauge>
          </div>
        </div>
      </section>

      <Panel title="Resolvers" subtitle={health.text} className={`dash-span ${health.degraded ? 'is-degraded' : ''}`}>
        {stats.upstreams.length === 0 ? (
          <EmptyState>No upstreams reported.</EmptyState>
        ) : (
          <div className="upstream-grid">
            {stats.upstreams.map((u) => (
              <UpstreamCard key={u.name} upstream={u} />
            ))}
          </div>
        )}
      </Panel>

      <Panel title="Queries and blocks" className="dash-span">
        {stats.timeline.length === 0 ? (
          <EmptyState>No traffic recorded in this range yet.</EmptyState>
        ) : (
          <LineChart points={stats.timeline} openLast />
        )}
      </Panel>

      <Panel title="Top domains" className="dash-cell">
        <BarList items={stats.top_domains} tone="cyan" emptyText={NO_TRAFFIC_HINT} maxRows={5} linkTo={queryLogLink} />
      </Panel>
      <Panel title="Root domains" subtitle="Each domain with all its subdomains" className="dash-cell">
        <BarList items={stats.root_domains ?? []} tone="cyan" emptyText={NO_TRAFFIC_HINT} maxRows={5} />
      </Panel>

      <Panel title="Blocked" subtitle="Click a domain to see its queries in the log" className="dash-span">
        <div className="blocked-split">
          <div>
            <h3 className="sub-head">Top blocked, with the list that blocked each</h3>
            <BarList
              items={stats.top_blocked}
              tone="red"
              emptyText="Nothing blocked yet. Either the network is quiet, or it is DNS."
              maxRows={5}
              linkTo={queryLogLink}
            />
          </div>
          <div>
            <h3 className="sub-head">Blocked queries by list</h3>
            <BarList items={stats.block_reasons ?? []} tone="red" emptyText="Nothing blocked yet." maxRows={5} />
          </div>
        </div>
      </Panel>

      <LiveLog className="dash-span" />

      <details className="panel insight dash-span">
        <summary>Traffic insight: who the queries go to, and where answers point</summary>
        <div className="dash-insights">
          <Panel title="GAFAM dominance" subtitle="Share of all queries by company" className="dash-cell">
            <CompanyDonut companies={stats.companies ?? []} total={stats.total} />
          </Panel>
          <Panel title="Traffic destination" subtitle="Countries the answers pointed to" className="dash-cell">
            <TrafficMap countries={stats.countries ?? []} geoReady={stats.geo_ready === true} />
          </Panel>
        </div>
      </details>
    </div>
  );
}

function UpstreamCard({ upstream }: { upstream: UpstreamStatus }) {
  // The probe median is the steady figure. Live averages only cover servers that have traffic.
  const probed = upstream.probe_p50_ms != null;
  const latencyMs = probed ? (upstream.probe_p50_ms as number) : upstream.avg_ms;
  const lat = splitMs(latencyMs);
  const statusColor = upstream.ok ? COLOR.green : COLOR.red;
  const latColor = latencyTone(latencyMs);
  return (
    <div className="upstream-card">
      <div className="upstream-head">
        <span className="upstream-name" title={upstream.url ?? upstream.name}>{upstream.name}</span>
        {upstream.protocol && <span className="badge badge-cyan">{PROTOCOL_LABEL[upstream.protocol]}</span>}
      </div>
      <div className="upstream-rings">
        <div className="mini">
          <Gauge value={upstream.ok ? 1 : 0} size={96} stroke={8} color={statusColor} label={`${upstream.name} status ${upstream.ok ? 'ok' : 'down'}`}>
            <span className="mini-text" style={{ color: statusColor }}>{upstream.ok ? 'OK' : 'DOWN'}</span>
          </Gauge>
          <span className="mini-caption">Status</span>
        </div>
        <div className="mini">
          <Gauge value={clamp01(latencyMs / 500)} size={96} stroke={8} color={latColor} label={`${upstream.name} ${probed ? 'probe median' : 'live average'} latency ${lat.value} ${lat.unit}`}>
            <span className="mini-text">{lat.value}<small> {lat.unit}</small></span>
          </Gauge>
          <span className="mini-caption">{probed ? 'Probe median' : 'Live average'}</span>
        </div>
      </div>
      <div className="upstream-foot">
        <span className={upstream.errors > 0 ? 'text-red' : 'muted'}>{formatInt(upstream.errors)} errors</span>
        {upstream.probe_p95_ms != null && (
          <span className="muted"> · p95 {Math.round(upstream.probe_p95_ms)} ms</span>
        )}
        {probed && upstream.avg_ms > 0 && (
          <span className="muted"> · live {Math.round(upstream.avg_ms)} ms</span>
        )}
      </div>
    </div>
  );
}

function LiveLog({ className = '' }: { className?: string }) {
  const { redactAddresses } = usePrefs();
  const log = useQueryLog({ limit: 12 }, true);

  return (
    <Panel
      title="Live query log"
      className={`live-log ${className}`.trim()}
      actions={<span className="live-dot" aria-label="Live">Live</span>}
    >
      {!log.data && log.isPending && <LoadingState label="Loading queries" />}
      {!log.data && log.isError && <ErrorState error={log.error} onRetry={() => log.refetch()} />}
      {log.data && (
        <QueryLogTable
          items={log.data.items}
          redact={redactAddresses}
          compact
          emptyText="No queries yet."
        />
      )}
    </Panel>
  );
}
