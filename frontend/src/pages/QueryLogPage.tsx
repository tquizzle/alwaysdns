import { useEffect, useState } from 'react';
import { useBlockDomain, useMe, useQueryLog } from '../api/queries';
import type { BlockBody, QueryLogEntry, QueryLogParams, ResultFilter } from '../api/types';
import { QueryLogTable } from '../components/QueryLogTable';
import { SegmentedControl } from '../components/SegmentedControl';
import { ErrorState, LoadingState, Notice, type NoticeState } from '../components/States';
import { formatInt, formatTime } from '../lib/format';
import { hashParam } from '../lib/hashRoute';
import { usePrefs } from '../lib/prefs';
import { RESULT_FILTERS, TIME_PRESETS, timeRangeToQuery, type TimePreset } from '../lib/querylog';
import { useDebouncedValue } from '../lib/useDebouncedValue';

const PAGE_SIZES = [50, 100];
const POLL_NOTE_SECONDS = 3;

export function QueryLogPage() {
  const { redactAddresses } = usePrefs();
  // Links from the dashboard arrive as #/querylog?domain=example.com.
  const [domain, setDomain] = useState(() => hashParam('domain') ?? '');
  const [client, setClient] = useState('');
  const [result, setResult] = useState<ResultFilter>('');
  const [preset, setPreset] = useState<TimePreset>('day');
  const [customFrom, setCustomFrom] = useState('');
  const [customTo, setCustomTo] = useState('');
  const [pageSize, setPageSize] = useState(50);
  const [pageIndex, setPageIndex] = useState(0);
  const [live, setLive] = useState(true);
  // Rows move when live data arrives, so refreshing waits while the pointer or focus is on the table.
  const [holding, setHolding] = useState(false);
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const me = useMe();
  const isAdmin = me.data?.role === 'admin';
  const block = useBlockDomain();

  const onBlock = (entry: QueryLogEntry, scope: 'device' | 'everyone') => {
    setNotice(null);
    const body: BlockBody =
      scope === 'device' ? { domain: entry.domain, client: entry.client } : { domain: entry.domain };
    block.mutate(body, {
      onSuccess: (result) => setNotice({ tone: 'ok', text: `Blocked ${result.domain} for ${result.scope}.` }),
      onError: (err) => setNotice({ tone: 'error', text: `Could not block ${entry.domain}: ${err.message}` }),
    });
  };
  const blockingDomain = block.isPending ? (block.variables?.domain ?? null) : null;

  // Text filters wait for a pause in typing before they reach the server.
  const debouncedDomain = useDebouncedValue(domain, 300);
  const debouncedClient = useDebouncedValue(client, 300);

  // Any filter change starts again from the first page.
  const goToFirstPage = () => setPageIndex(0);

  const bounds = timeRangeToQuery({ preset, customFrom, customTo }, Date.now());
  const offset = pageIndex * pageSize;
  const isFirstPage = offset === 0;

  const params: QueryLogParams = {
    limit: pageSize,
    offset,
    domain: debouncedDomain.trim() || undefined,
    client: debouncedClient.trim() || undefined,
    result: result || undefined,
    from: bounds.from,
    to: bounds.to,
  };

  // Live refresh only applies to the first page with no offset.
  const log = useQueryLog(params, live && isFirstPage && !holding, bounds.error === undefined);

  const items = log.data?.items ?? [];
  const total = log.data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const firstShown = total === 0 ? 0 : offset + 1;
  const lastShown = offset + items.length;

  // A live refresh can shrink the result set. Keep the page inside the range.
  useEffect(() => {
    if (log.data && total > 0 && offset >= total) {
      setPageIndex(Math.ceil(total / pageSize) - 1);
    }
  }, [log.data, total, offset, pageSize]);

  const liveNote = !live
    ? 'Live updates paused.'
    : !isFirstPage
      ? 'Live updates apply to the first page only.'
      : holding
        ? 'Paused while you use the table.'
        : `Refreshing every ${POLL_NOTE_SECONDS} seconds.`;

  return (
    <div className="page">
      <div className="page-bar">
        <div>
          <h1>Query log</h1>
          <p className="muted small">
            {liveNote}
            {log.dataUpdatedAt && live && isFirstPage ? ` Last update ${formatTime(log.dataUpdatedAt)}.` : ''}
            {log.isError && log.data ? ' Could not refresh, showing the last results.' : ''}
          </p>
        </div>
        <button
          type="button"
          className={`btn live-toggle ${live ? 'is-live' : ''}`}
          aria-pressed={live}
          onClick={() => setLive((v) => !v)}
        >
          <span className="live-dot-icon" aria-hidden="true" />
          {live ? 'Live on' : 'Live off'}
        </button>
      </div>

      <div className="filters panel" aria-label="Query filters">
        <label className="field">
          <span>Domain</span>
          <input
            type="search"
            placeholder="e.g. example.com"
            value={domain}
            onChange={(e) => {
              setDomain(e.target.value);
              goToFirstPage();
            }}
            spellCheck={false}
            autoCapitalize="none"
          />
        </label>
        <label className="field">
          <span>Client</span>
          <input
            type="search"
            placeholder="Client name or IP"
            value={client}
            onChange={(e) => {
              setClient(e.target.value);
              goToFirstPage();
            }}
            spellCheck={false}
          />
        </label>
        <label className="field narrow">
          <span>Result</span>
          <select
            value={result}
            onChange={(e) => {
              setResult(e.target.value as ResultFilter);
              goToFirstPage();
            }}
          >
            {RESULT_FILTERS.map((option) => (
              <option key={option.value || 'all'} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <div className="field time-field">
          <span>Time range</span>
          <SegmentedControl
            label="Time range"
            options={TIME_PRESETS}
            value={preset}
            onChange={(value) => {
              setPreset(value);
              goToFirstPage();
            }}
          />
        </div>
        {preset === 'custom' && (
          <div className="custom-range">
            <label className="field">
              <span>From</span>
              <input
                type="datetime-local"
                value={customFrom}
                onChange={(e) => {
                  setCustomFrom(e.target.value);
                  goToFirstPage();
                }}
              />
            </label>
            <label className="field">
              <span>To</span>
              <input
                type="datetime-local"
                value={customTo}
                onChange={(e) => {
                  setCustomTo(e.target.value);
                  goToFirstPage();
                }}
              />
            </label>
          </div>
        )}
        <label className="field narrow">
          <span>Page size</span>
          <select
            value={pageSize}
            onChange={(e) => {
              setPageSize(Number(e.target.value));
              goToFirstPage();
            }}
          >
            {PAGE_SIZES.map((n) => (
              <option key={n} value={n}>{n}</option>
            ))}
          </select>
        </label>
      </div>

      <div className="panel">
        {bounds.error && <Notice tone="error">{bounds.error}</Notice>}

        {!bounds.error && !log.data && log.isPending && <LoadingState label="Loading queries" />}
        {!bounds.error && !log.data && log.isError && (
          <ErrorState error={log.error} onRetry={() => log.refetch()} />
        )}

        {!bounds.error && log.data && (
          <>
            <div className="pager-bar">
              <p className="muted small count-line">
                {total === 0
                  ? 'No matching entries'
                  : `Showing ${formatInt(firstShown)} to ${formatInt(lastShown)} of ${formatInt(total)} ${total === 1 ? 'entry' : 'entries'}`}
                {log.isFetching ? ' · updating' : ''}
              </p>
              <nav className="pager" aria-label="Query log pages">
                <button
                  type="button"
                  className="btn btn-small"
                  disabled={pageIndex === 0}
                  onClick={() => setPageIndex((p) => Math.max(0, p - 1))}
                >
                  Previous
                </button>
                <span className="muted small">Page {formatInt(pageIndex + 1)} of {formatInt(pageCount)}</span>
                <button
                  type="button"
                  className="btn btn-small"
                  disabled={offset + pageSize >= total}
                  onClick={() => setPageIndex((p) => p + 1)}
                >
                  Next
                </button>
              </nav>
            </div>
            {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}
            <div
              onMouseEnter={() => setHolding(true)}
              onMouseLeave={() => setHolding(false)}
              onFocus={() => setHolding(true)}
              onBlur={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget)) setHolding(false);
              }}
            >
              <QueryLogTable
                items={items}
                redact={redactAddresses}
                pagination={{ pageIndex, pageSize, pageCount }}
                emptyText="No queries match these filters."
                onBlock={isAdmin ? onBlock : undefined}
                blockingDomain={blockingDomain}
              />
            </div>
          </>
        )}
      </div>
    </div>
  );
}
