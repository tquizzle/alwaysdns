import { createColumnHelper, type SortingState } from '@tanstack/react-table';
import { useMemo, useState } from 'react';
import type { QueryLogEntry } from '../api/types';
import { formatDateTime, formatMs, formatTime, orDash, parseTimestamp } from '../lib/format';
import { redactDisplay } from '../lib/redact';
import { DataTable, type PaginationState } from './DataTable';

const helper = createColumnHelper<QueryLogEntry>();

export function resultClass(result: string): string {
  switch (result.toLowerCase()) {
    case 'blocked':
      return 'badge badge-red';
    case 'cached':
      return 'badge badge-cyan';
    case 'forwarded':
    case 'allowed':
      return 'badge badge-green';
    case 'rewritten':
      return 'badge badge-yellow';
    case 'error':
      return 'badge badge-error';
    default:
      return 'badge';
  }
}

interface Props {
  items: QueryLogEntry[];
  redact: boolean;
  compact?: boolean;
  /** Server-side paging. Turns off client sorting, since the server orders the rows. */
  pagination?: PaginationState;
  emptyText?: string;
  /** Admin only: adds a Block column. `device` blocks for this client, `everyone` for all devices. */
  onBlock?: (entry: QueryLogEntry, scope: 'device' | 'everyone') => void;
  /** Domain whose block request is in flight, to disable its buttons. */
  blockingDomain?: string | null;
}

const SORTING: SortingState = [{ id: 'time', desc: true }];

/** Identity of a log row, stable across live refreshes that shift rows down the page. */
function entryKey(entry: QueryLogEntry): string {
  return `${entry.time}|${entry.client}|${entry.domain}|${entry.qtype}`;
}

interface BlockCellProps {
  entry: QueryLogEntry;
  armed: boolean;
  busy: boolean;
  onArm: (open: boolean) => void;
  onBlock: (entry: QueryLogEntry, scope: 'device' | 'everyone') => void;
}

/** One quiet Block button per row. It opens a choice of scope, so a stray click never blocks anything. */
function BlockCell({ entry, armed, busy, onArm, onBlock }: BlockCellProps) {
  // "unknown" means no configured client matched, so there is no device to block for.
  const hasDevice = entry.client !== 'unknown' && entry.client !== '';
  if (!armed) {
    return (
      <button
        type="button"
        className="btn btn-small"
        disabled={busy}
        aria-label={`Block ${entry.domain}`}
        aria-haspopup="true"
        onClick={() => onArm(true)}
      >
        Block…
      </button>
    );
  }
  const choose = (scope: 'device' | 'everyone') => {
    onArm(false);
    onBlock(entry, scope);
  };
  return (
    <div className="row-actions" role="group" aria-label={`Block ${entry.domain} for`}>
      <button
        type="button"
        className="btn btn-small btn-danger-outline"
        disabled={!hasDevice}
        aria-label={`Block ${entry.domain} for ${entry.client} only`}
        title={hasDevice ? undefined : 'No configured device matched this query'}
        onClick={() => choose('device')}
      >
        This device
      </button>
      <button
        type="button"
        className="btn btn-small btn-danger-outline"
        aria-label={`Block ${entry.domain} for every device`}
        onClick={() => choose('everyone')}
      >
        Everyone
      </button>
      <button type="button" className="btn btn-small" aria-label="Cancel blocking" onClick={() => onArm(false)}>
        Cancel
      </button>
    </div>
  );
}

export function QueryLogTable({
  items,
  redact,
  compact = false,
  pagination,
  emptyText = 'No queries match these filters yet.',
  onBlock,
  blockingDomain = null,
}: Props) {
  const [armedKey, setArmedKey] = useState<string | null>(null);
  const columns = useMemo(
    () => [
      helper.accessor('time', {
        header: 'Time',
        cell: (info) => (
          <span title={formatDateTime(info.getValue())}>{formatTime(info.getValue())}</span>
        ),
        sortingFn: (a, b) => {
          const av = parseTimestamp(a.original.time)?.getTime() ?? 0;
          const bv = parseTimestamp(b.original.time)?.getTime() ?? 0;
          return av === bv ? 0 : av > bv ? 1 : -1;
        },
      }),
      helper.accessor('client', {
        header: 'Client',
        cell: (info) => <span className="mono">{redactDisplay(info.getValue(), redact)}</span>,
      }),
      helper.accessor('domain', {
        header: 'Domain',
        cell: (info) => <span className="mono domain" title={info.getValue()}>{info.getValue()}</span>,
      }),
      helper.accessor('qtype', {
        header: 'Type',
        cell: (info) => <span className="mono">{orDash(info.getValue())}</span>,
      }),
      helper.accessor('result', {
        header: 'Result',
        cell: (info) => <span className={resultClass(info.getValue())}>{orDash(info.getValue())}</span>,
      }),
      helper.accessor('upstream', {
        header: 'Upstream',
        cell: (info) => {
          const entry = info.row.original;
          return <span title={entry.upstream}>{orDash(entry.upstream_name ?? entry.upstream)}</span>;
        },
      }),
      helper.accessor('elapsed_ms', {
        header: 'Latency',
        cell: (info) => <span className="mono">{formatMs(info.getValue())}</span>,
      }),
      helper.accessor('rule', {
        header: 'Rule',
        cell: (info) => <span className="muted">{orDash(info.getValue())}</span>,
      }),
    ],
    [redact],
  );

  const allColumns = useMemo(() => {
    if (!onBlock) return columns;
    return [
      ...columns,
      helper.display({
        id: 'block',
        header: 'Block',
        cell: ({ row }) => {
          const entry = row.original;
          const key = entryKey(entry);
          return (
            <BlockCell
              entry={entry}
              armed={armedKey === key}
              busy={blockingDomain === entry.domain}
              onArm={(open) => setArmedKey(open ? key : null)}
              onBlock={onBlock}
            />
          );
        },
      }),
    ];
  }, [columns, onBlock, blockingDomain, armedKey]);

  return (
    <DataTable
      data={items}
      columns={allColumns}
      initialSorting={SORTING}
      sortable={!pagination}
      pagination={pagination}
      compact={compact}
      emptyText={emptyText}
      label="DNS query log"
    />
  );
}
