import { useState } from 'react';
import {
  useAssignList,
  useClients,
  useDeleteList,
  useLists,
  useRefreshLists,
  useUpdateList,
} from '../api/queries';
import type { FilterList, ListKind } from '../api/types';
import { ListDialog } from '../components/ListDialog';
import { ListForm } from '../components/ListForm';
import { ListsTable } from '../components/ListsTable';
import { Notice, type NoticeState, ErrorState, LoadingState } from '../components/States';
import { Panel } from '../components/Panel';
import { LIST_KIND_LABEL } from '../lib/lists';
import { usePrefs } from '../lib/prefs';

const TABS: ListKind[] = ['block', 'allow'];

interface Props {
  isAdmin: boolean;
}

export function ListsPage({ isAdmin }: Props) {
  const lists = useLists();
  const { redactAddresses } = usePrefs();
  const [kind, setKind] = useState<ListKind>('block');
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const [editing, setEditing] = useState<FilterList | null>(null);
  const [confirmId, setConfirmId] = useState<number | null>(null);

  const refresh = useRefreshLists();
  const toggle = useUpdateList();
  const remove = useDeleteList();
  const assign = useAssignList();
  const clients = useClients();
  const clientNames = clients.data?.clients.map((c) => c.name) ?? [];
  const assigningName = assign.isPending ? (assign.variables?.name ?? null) : null;

  const onRefresh = () => {
    setNotice(null);
    refresh.mutate(undefined, {
      onSuccess: (result) =>
        setNotice({ tone: 'ok', text: `Refreshed ${result.lists} ${result.lists === 1 ? 'list' : 'lists'}.` }),
      onError: (err) => setNotice({ tone: 'error', text: err.message }),
    });
  };

  const onToggle = (list: FilterList) => {
    setNotice(null);
    toggle.mutate(
      { id: list.id, body: { enabled: !list.enabled } },
      { onError: (err) => setNotice({ tone: 'error', text: `Could not update ${list.name}: ${err.message}` }) },
    );
  };

  const onConfirmDelete = (list: FilterList) => {
    setNotice(null);
    remove.mutate(list.id, {
      onSuccess: () => setNotice({ tone: 'ok', text: `Deleted ${list.name}.` }),
      onError: (err) => setNotice({ tone: 'error', text: `Could not delete ${list.name}: ${err.message}` }),
      onSettled: () => setConfirmId(null),
    });
  };

  const onAssign = (list: FilterList, clients: string[], isDefault: boolean) => {
    setNotice(null);
    assign.mutate(
      { name: list.name, clients, default: isDefault },
      {
        onSuccess: () => setNotice({ tone: 'ok', text: `Updated devices for ${list.name}.` }),
        onError: (err) => setNotice({ tone: 'error', text: `Could not assign ${list.name}: ${err.message}` }),
      },
    );
  };

  const togglingId = toggle.isPending ? (toggle.variables?.id ?? null) : null;
  const deletingId = remove.isPending ? (remove.variables ?? null) : null;

  return (
    <div className="page">
      <div className="page-bar">
        <div>
          <h1>Lists</h1>
          <p className="muted small">
            {isAdmin
              ? 'Block and allow lists. Refresh them to pull the latest rules.'
              : 'Block and allow lists. Read only for your account.'}
          </p>
        </div>
        {isAdmin && (
          <button type="button" className="btn btn-primary" onClick={onRefresh} disabled={refresh.isPending}>
            {refresh.isPending ? 'Refreshing...' : 'Refresh all'}
          </button>
        )}
      </div>

      {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}

      {!lists.data && lists.isPending && <LoadingState label="Loading lists" />}
      {!lists.data && lists.isError && <ErrorState error={lists.error} onRetry={() => lists.refetch()} />}

      {lists.data && (
        <>
          <div className="tabs" role="tablist" aria-label="List kind">
            {TABS.map((tab) => {
              const selected = tab === kind;
              return (
                <button
                  key={tab}
                  type="button"
                  role="tab"
                  id={`lists-tab-${tab}`}
                  aria-selected={selected}
                  aria-controls="lists-tabpanel"
                  tabIndex={selected ? 0 : -1}
                  className={selected ? 'tab active' : 'tab'}
                  onClick={() => setKind(tab)}
                >
                  {LIST_KIND_LABEL[tab]}
                  <span className="tab-count">{lists.data[tab].length}</span>
                </button>
              );
            })}
          </div>

          <div role="tabpanel" id="lists-tabpanel" aria-labelledby={`lists-tab-${kind}`} className="page">
            {isAdmin && (
              <ListForm
                kind={kind}
                onAdded={(text) => setNotice({ tone: 'ok', text })}
              />
            )}

            <Panel title={LIST_KIND_LABEL[kind]}>
              <ListsTable
                lists={lists.data[kind]}
                isAdmin={isAdmin}
                maskUrls={!isAdmin || redactAddresses}
                clientNames={clientNames}
                assigningName={assigningName}
                onAssign={onAssign}
                togglingId={togglingId}
                confirmDeleteId={confirmId}
                deletingId={deletingId}
                onToggle={onToggle}
                onEdit={setEditing}
                onAskDelete={setConfirmId}
                onCancelDelete={() => setConfirmId(null)}
                onConfirmDelete={(id) => {
                  const list = lists.data[kind].find((l) => l.id === id);
                  if (list) onConfirmDelete(list);
                }}
              />
            </Panel>
          </div>
        </>
      )}

      {editing && (
        <ListDialog
          list={editing}
          onClose={() => setEditing(null)}
          onSaved={(text) => setNotice({ tone: 'ok', text })}
        />
      )}
    </div>
  );
}
