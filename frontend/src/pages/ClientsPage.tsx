import { useCallback, useMemo, useState } from 'react';
import { useClients, useDeleteClient, useLists, useMe, useUpdateClient } from '../api/queries';
import type { Client, ClientUpdateBody } from '../api/types';
import { ClientDialog } from '../components/ClientDialog';
import { ClientsTable } from '../components/ClientsTable';
import { Panel } from '../components/Panel';
import { ErrorState, LoadingState, Notice, type NoticeState } from '../components/States';
import { formatInt } from '../lib/format';
import { usePrefs } from '../lib/prefs';

export function ClientsPage() {
  const { redactAddresses } = usePrefs();
  const me = useMe();
  const isAdmin = me.data?.role === 'admin';
  const clients = useClients();
  const lists = useLists();
  const update = useUpdateClient();
  const remove = useDeleteClient();
  const [notice, setNotice] = useState<NoticeState | null>(null);
  // 'new' adds a client. A client edits its rules.
  const [editing, setEditing] = useState<Client | 'new' | null>(null);

  const groupNames = useMemo(
    () => (clients.data?.upstream_groups ?? []).map((g) => g.name),
    [clients.data],
  );
  const defaultGroup = useMemo(
    () => clients.data?.upstream_groups.find((g) => g.is_default)?.name ?? groupNames[0] ?? '',
    [clients.data, groupNames],
  );
  const blockListNames = useMemo(
    () => [...new Set((lists.data?.block ?? []).map((l) => l.name))],
    [lists.data],
  );
  const savingName = update.isPending ? (update.variables?.name ?? null) : null;
  const deletingName = remove.isPending ? (remove.variables ?? null) : null;

  const onUpdate = useCallback(
    (client: Client, body: ClientUpdateBody) => {
      setNotice(null);
      update.mutate(
        { name: client.name, body },
        {
          onSuccess: () => setNotice({ tone: 'ok', text: `Saved ${client.name}.` }),
          onError: (err) => setNotice({ tone: 'error', text: `Could not save ${client.name}: ${err.message}` }),
        },
      );
    },
    [update],
  );

  const onDelete = useCallback(
    (client: Client) => {
      setNotice(null);
      remove.mutate(client.name, {
        onSuccess: () => setNotice({ tone: 'ok', text: `Deleted ${client.name}.` }),
        onError: (err) => setNotice({ tone: 'error', text: `Could not delete ${client.name}: ${err.message}` }),
      });
    },
    [remove],
  );

  return (
    <div className="page">
      <div className="page-bar">
        <div>
          <h1>Clients</h1>
          <p className="muted small">
            {isAdmin
              ? 'Devices and networks, with the rules each one uses.'
              : 'Devices and networks grouped by upstream. Read only for your account.'}
          </p>
        </div>
      </div>

      {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}

      {!clients.data && clients.isPending && <LoadingState label="Loading clients" />}
      {!clients.data && clients.isError && <ErrorState error={clients.error} onRetry={() => clients.refetch()} />}
      {clients.data && (
        <>
          <Panel
            title="Configured clients"
            actions={
              <div className="row-actions">
                <span className="muted small">
                  {formatInt(clients.data.clients.length)} clients · {formatInt(clients.data.clients.reduce((s, c) => s + c.queries, 0))} queries
                </span>
                {isAdmin && (
                  <button type="button" className="btn btn-small btn-primary" onClick={() => setEditing('new')}>
                    Add client
                  </button>
                )}
              </div>
            }
          >
            <ClientsTable
              clients={clients.data.clients}
              redact={redactAddresses}
              isAdmin={isAdmin}
              groupNames={groupNames}
              blockListNames={blockListNames}
              savingName={savingName}
              onUpdate={onUpdate}
              onEdit={setEditing}
              onDelete={onDelete}
              deletingName={deletingName}
            />
          </Panel>

          {isAdmin && (
            <p className="muted small">
              Resolvers and upstream groups are set up on the <a href="#/upstreams">Upstreams</a> page.
            </p>
          )}
        </>
      )}

      {isAdmin && editing !== null && (
        <ClientDialog
          client={editing === 'new' ? null : editing}
          groupNames={groupNames}
          defaultGroup={defaultGroup}
          blockListNames={blockListNames}
          onClose={() => setEditing(null)}
          onSaved={(text) => {
            setEditing(null);
            setNotice({ tone: 'ok', text });
          }}
        />
      )}
    </div>
  );
}
