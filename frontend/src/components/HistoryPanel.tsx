import { useState, type FormEvent } from 'react';
import { useHistory } from '../api/queries';
import type { Bucket } from '../api/types';
import { EmptyState, ErrorState, LoadingState, Notice } from './States';
import { LineChart } from './LineChart';
import { Panel } from './Panel';
import { BUCKET_OPTIONS, bucketLabel, formatBucketTick, historyParams } from '../lib/history';
import { toLocalInputValue } from '../lib/timeInput';

const DAY_MS = 24 * 60 * 60 * 1000;

interface Form {
  from: string;
  to: string;
  bucket: Bucket;
}

/** Date range picker and timeline for a chosen window, from /api/history. */
export function HistoryPanel() {
  // Default to the last 7 days, read once so the inputs do not move while typing.
  const [initial] = useState<Form>(() => {
    const now = Date.now();
    return { from: toLocalInputValue(now - 7 * DAY_MS), to: toLocalInputValue(now), bucket: 'hour' };
  });
  const [form, setForm] = useState<Form>(initial);
  const [applied, setApplied] = useState<Form>(initial);

  const formError = historyParams(form.from, form.to, form.bucket).error;
  const request = historyParams(applied.from, applied.to, applied.bucket).params;
  const history = useHistory(request);

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (formError) return;
    setApplied(form);
  };

  return (
    <Panel
      title="History"
      actions={<span className="muted small">{bucketLabel(applied.bucket)}</span>}
      className="history-panel"
    >
      <form className="history-form" onSubmit={onSubmit} noValidate aria-label="History date range">
        <label className="field">
          <span>From</span>
          <input
            type="datetime-local"
            value={form.from}
            onChange={(e) => setForm((prev) => ({ ...prev, from: e.target.value }))}
          />
        </label>
        <label className="field">
          <span>To</span>
          <input
            type="datetime-local"
            value={form.to}
            onChange={(e) => setForm((prev) => ({ ...prev, to: e.target.value }))}
          />
        </label>
        <label className="field narrow">
          <span>Bucket</span>
          <select
            value={form.bucket}
            onChange={(e) => setForm((prev) => ({ ...prev, bucket: e.target.value as Bucket }))}
          >
            {BUCKET_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <button type="submit" className="btn btn-primary" disabled={formError !== null}>
          Show history
        </button>
      </form>

      {formError && <Notice tone="error">{formError}</Notice>}

      {request && history.isPending && <LoadingState label="Loading history" />}
      {history.isError && <ErrorState error={history.error} onRetry={() => history.refetch()} />}
      {history.data && (
        history.data.timeline.length === 0 ? (
          <EmptyState>No traffic in this period.</EmptyState>
        ) : (
          <LineChart
            points={history.data.timeline}
            formatTick={(t) => formatBucketTick(Number(t), applied.bucket)}
          />
        )
      )}
    </Panel>
  );
}
