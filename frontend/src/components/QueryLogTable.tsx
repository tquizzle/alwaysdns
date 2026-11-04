import { createColumnHelper, type SortingState } from '@tanstack/react-table';
import { useMemo } from 'react';
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
      return 'badge badge-red';
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

export function QueryLogTable({
  items,
  redact,
  compact = false,
  pagination,
  emptyText = 'No queries match these filters yet.',
  onBlock,
  blockingDomain = null,
}: Props) {
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
        cell: (info) => orDash(info.getValue()),
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
          const busy = blockingDomain === entry.domain;
          // "unknown" means no configured client matched, so there is no device to block for.
          const hasDevice = entry.client !== 'unknown' && entry.client !== '';
          return (
            <div className="row-actions">
              <button
                type="button"
                className="btn btn-small btn-danger-outline"
                disabled={busy || !hasDevice}
                title={hasDevice ? `Block ${entry.domain} for ${entry.client}` : 'No configured device matched this query'}
                onClick={() => onBlock(entry, 'device')}
              >
                This device
              </button>
              <button
                type="button"
                className="btn btn-small btn-danger-outline"
                disabled={busy}
                title={`Block ${entry.domain} for every device`}
                onClick={() => onBlock(entry, 'everyone')}
              >
                Everyone
              </button>
            </div>
          );
        },
      }),
    ];
  }, [columns, onBlock, blockingDomain]);

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
