import { createColumnHelper } from '@tanstack/react-table';
import { useMemo, useState } from 'react';
import type { Client, ClientUpdateBody } from '../api/types';
import { clientBody } from '../lib/clients';
import { formatInt, orDash } from '../lib/format';
import { redactAddressList } from '../lib/redact';
import { DataTable } from './DataTable';

const helper = createColumnHelper<Client>();

interface Props {
  clients: Client[];
  redact: boolean;
  /** Admins can change rules, and add or delete clients. Viewers see read-only values. */
  isAdmin: boolean;
  groupNames: string[];
  /** Every block list name, in the order to show them. */
  blockListNames: string[];
  /** Name of the client being saved right now, or null. */
  savingName: string | null;
  /** Saves the full rule set of one client. */
  onUpdate: (client: Client, body: ClientUpdateBody) => void;
  onEdit: (client: Client) => void;
  onDelete: (client: Client) => void;
  deletingName: string | null;
}

interface BlockChipsProps {
  client: Client;
  blockListNames: string[];
  busy: boolean;
  onUpdate: Props['onUpdate'];
}

/** Toggles which block lists a client uses. Each click saves right away. */
function BlockChips({ client, blockListNames, busy, onUpdate }: BlockChipsProps) {
  const assigned = new Set(client.block_lists);
  const toggle = (name: string) => {
    const next = new Set(assigned);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    onUpdate(client, { ...clientBody(client), block_lists: [...next] });
  };

  if (blockListNames.length === 0) return <span className="muted">No block lists yet</span>;
  return (
    <div className="chip-row" role="group" aria-label={`Block lists for ${client.name}`}>
      {blockListNames.map((name) => (
        <button
          key={name}
          type="button"
          className={`chip ${assigned.has(name) ? 'is-on' : ''}`}
          aria-pressed={assigned.has(name)}
          disabled={busy}
          onClick={() => toggle(name)}
        >
          {name}
        </button>
      ))}
    </div>
  );
}

interface ActionsProps {
  client: Client;
  deleting: boolean;
  onEdit: (client: Client) => void;
  onDelete: (client: Client) => void;
}

/** Edit opens the rules dialog. Delete asks for a second click first. */
function RowActions({ client, deleting, onEdit, onDelete }: ActionsProps) {
  const [confirming, setConfirming] = useState(false);
  if (confirming) {
    return (
      <div className="row-actions">
        <span className="small text-red">Delete {client.name}?</span>
        <button
          type="button"
          className="btn btn-small btn-danger"
          disabled={deleting}
          onClick={() => onDelete(client)}
        >
          {deleting ? 'Deleting...' : 'Confirm'}
        </button>
        <button type="button" className="btn btn-small" onClick={() => setConfirming(false)}>
          Cancel
        </button>
      </div>
    );
  }
  return (
    <div className="row-actions">
      <button type="button" className="btn btn-small" onClick={() => onEdit(client)}>
        Edit
      </button>
      <button type="button" className="btn btn-small btn-danger-outline" onClick={() => setConfirming(true)}>
        Delete
      </button>
    </div>
  );
}

export function ClientsTable({
  clients,
  redact,
  isAdmin,
  groupNames,
  blockListNames,
  savingName,
  onUpdate,
  onEdit,
  onDelete,
  deletingName,
}: Props) {
  const columns = useMemo(() => {
    const base = [
      helper.accessor('name', {
        header: 'Client',
        cell: (info) => <strong>{info.getValue()}</strong>,
      }),
      helper.accessor('cidrs', {
        header: 'Networks',
        enableSorting: false,
        cell: (info) => {
          const cidrs = info.getValue() ?? [];
          if (cidrs.length === 0) return <span className="muted">-</span>;
          return <span className="mono">{redactAddressList(cidrs, redact).join(', ')}</span>;
        },
      }),
    ];

    const group = isAdmin
      ? helper.accessor('upstream_group', {
          header: 'Upstream group',
          cell: (info) => {
            const client = info.row.original;
            const current = info.getValue();
            // Keep the current value selectable even if it is no longer in the list.
            const options = groupNames.includes(current) ? groupNames : [current, ...groupNames];
            return (
              <select
                className="client-select"
                aria-label={`Upstream group for ${client.name}`}
                value={current}
                disabled={savingName === client.name}
                onChange={(e) => onUpdate(client, { ...clientBody(client), upstream_group: e.target.value })}
              >
                {options.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            );
          },
        })
      : helper.accessor('upstream_group', {
          header: 'Upstream group',
          cell: (info) => orDash(info.getValue()),
        });

    const lists = isAdmin
      ? helper.display({
          id: 'block_lists',
          header: 'Block lists',
          cell: ({ row }) => (
            <BlockChips
              client={row.original}
              blockListNames={blockListNames}
              busy={savingName === row.original.name}
              onUpdate={onUpdate}
            />
          ),
        })
      : helper.accessor('block_lists', {
          header: 'Block lists',
          enableSorting: false,
          cell: (info) => {
            const names = info.getValue();
            return names.length === 0 ? <span className="muted">-</span> : names.join(', ');
          },
        });

    const actions = isAdmin
      ? helper.display({
          id: 'actions',
          header: 'Actions',
          cell: ({ row }) => (
            <RowActions
              client={row.original}
              deleting={deletingName === row.original.name}
              onEdit={onEdit}
              onDelete={onDelete}
            />
          ),
        })
      : null;

    return [
      ...base,
      group,
      lists,
      helper.accessor('queries', {
        header: 'Queries',
        sortDescFirst: true,
        cell: (info) => <span className="mono">{formatInt(info.getValue())}</span>,
      }),
      ...(actions ? [actions] : []),
    ];
  }, [redact, isAdmin, groupNames, blockListNames, savingName, deletingName, onUpdate, onEdit, onDelete]);

  return (
    <DataTable
      data={clients}
      columns={columns}
      initialSorting={[{ id: 'queries', desc: true }]}
      emptyText={isAdmin ? 'No clients yet. Add one above.' : 'No clients configured.'}
      label="Configured clients"
    />
  );
}
