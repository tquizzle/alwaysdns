import { createColumnHelper } from '@tanstack/react-table';
import { useMemo } from 'react';
import type { FilterList } from '../api/types';
import { formatDateTime, formatInt } from '../lib/format';
import { redactListUrl } from '../lib/lists';
import { ConfirmDelete } from './ConfirmDelete';
import { DataTable } from './DataTable';

const helper = createColumnHelper<FilterList>();

interface Props {
  lists: FilterList[];
  isAdmin: boolean;
  /** Mask the URL host and query. Viewers always get the masked form. */
  maskUrls: boolean;
  togglingId: number | null;
  confirmDeleteId: number | null;
  deletingId: number | null;
  onToggle: (list: FilterList) => void;
  onEdit: (list: FilterList) => void;
  onAskDelete: (id: number) => void;
  onCancelDelete: () => void;
  onConfirmDelete: (id: number) => void;
  /** Configured client names, shown as chips on each block list. */
  clientNames: string[];
  /** Name of the block list being assigned right now, or null. */
  assigningName: string | null;
  onAssign: (list: FilterList, clients: string[], isDefault: boolean) => void;
}

interface AssignChipsProps {
  list: FilterList;
  clientNames: string[];
  busy: boolean;
  onAssign: (list: FilterList, clients: string[], isDefault: boolean) => void;
}

/** Which devices block with a block list: unknown devices, plus each configured client. */
function AssignChips({ list, clientNames, busy, onAssign }: AssignChipsProps) {
  if (list.kind !== 'block') return <span className="muted">-</span>;
  const assigned = new Set(list.assigned_clients ?? []);
  const isDefault = list.assigned_default === true;

  const toggleClient = (name: string) => {
    const next = new Set(assigned);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    onAssign(list, [...next], isDefault);
  };

  return (
    <div className="chip-row" role="group" aria-label={`Devices using ${list.name}`}>
      <button
        type="button"
        className={`chip ${isDefault ? 'is-on' : ''}`}
        aria-pressed={isDefault}
        disabled={busy}
        onClick={() => onAssign(list, [...assigned], !isDefault)}
      >
        Unknown devices
      </button>
      {clientNames.map((name) => (
        <button
          key={name}
          type="button"
          className={`chip ${assigned.has(name) ? 'is-on' : ''}`}
          aria-pressed={assigned.has(name)}
          disabled={busy}
          onClick={() => toggleClient(name)}
        >
          {name}
        </button>
      ))}
    </div>
  );
}

export function ListsTable({
  lists,
  isAdmin,
  maskUrls,
  togglingId,
  confirmDeleteId,
  deletingId,
  onToggle,
  onEdit,
  onAskDelete,
  onCancelDelete,
  onConfirmDelete,
  clientNames,
  assigningName,
  onAssign,
}: Props) {
  const columns = useMemo(() => {
    const base = [
      helper.accessor('name', {
        header: 'Name',
        cell: (info) => <strong>{info.getValue()}</strong>,
      }),
      helper.accessor('url', {
        header: 'URL',
        enableSorting: false,
        cell: (info) => {
          const shown = maskUrls ? redactListUrl(info.getValue()) : info.getValue();
          return (
            <span className="mono url-text" title={maskUrls ? undefined : shown}>
              {shown}
            </span>
          );
        },
      }),
      helper.accessor('enabled', {
        header: 'Enabled',
        enableSorting: false,
        cell: (info) => {
          const list = info.row.original;
          if (!isAdmin) {
            return <span className={list.enabled ? 'badge badge-green' : 'badge'}>{list.enabled ? 'On' : 'Off'}</span>;
          }
          return (
            <button
              type="button"
              role="switch"
              aria-checked={list.enabled}
              aria-label={`${list.name} enabled`}
              className={`switch ${list.enabled ? 'is-on' : ''}`}
              disabled={togglingId === list.id}
              onClick={() => onToggle(list)}
            >
              <span className="switch-knob" aria-hidden="true" />
            </button>
          );
        },
      }),
      helper.accessor('rules', {
        header: 'Rules',
        cell: (info) => <span className="mono">{formatInt(info.getValue())}</span>,
      }),
      helper.accessor('last_updated', {
        header: 'Last updated',
        cell: (info) => <span className="mono">{formatDateTime(info.getValue())}</span>,
      }),
      helper.accessor('error', {
        header: 'Error',
        enableSorting: false,
        cell: (info) => {
          const message = info.getValue();
          return message ? (
            <span className="text-red list-error">{message}</span>
          ) : (
            <span className="muted">-</span>
          );
        },
      }),
    ];

    if (!isAdmin) return base;

    return [
      ...base,
      helper.display({
        id: 'assigned',
        header: 'Assigned to',
        cell: ({ row }) => (
          <AssignChips
            list={row.original}
            clientNames={clientNames}
            busy={assigningName === row.original.name}
            onAssign={onAssign}
          />
        ),
      }),
      helper.display({
        id: 'actions',
        header: 'Actions',
        cell: ({ row }) => {
          const list = row.original;
          if (confirmDeleteId === list.id) {
            return (
              <ConfirmDelete
                prompt="Delete this list?"
                deleting={deletingId === list.id}
                onConfirm={() => onConfirmDelete(list.id)}
                onCancel={onCancelDelete}
              />
            );
          }
          return (
            <div className="row-actions">
              <button type="button" className="btn btn-small" onClick={() => onEdit(list)}>
                Edit
              </button>
              <button type="button" className="btn btn-small btn-danger-outline" onClick={() => onAskDelete(list.id)}>
                Delete
              </button>
            </div>
          );
        },
      }),
    ];
  }, [
    isAdmin,
    maskUrls,
    togglingId,
    confirmDeleteId,
    deletingId,
    onToggle,
    onEdit,
    onAskDelete,
    onCancelDelete,
    onConfirmDelete,
    clientNames,
    assigningName,
    onAssign,
  ]);

  return (
    <DataTable
      data={lists}
      columns={columns}
      initialSorting={[{ id: 'name', desc: false }]}
      emptyText={isAdmin ? 'No lists here yet. Add one above.' : 'No lists here yet.'}
      label="Filter lists"
    />
  );
}
