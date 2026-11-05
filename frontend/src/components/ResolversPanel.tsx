import { useEffect, useRef, useState, type FormEvent } from 'react';
import { useCreateResolver, useDeleteResolver, useUpdateResolver } from '../api/queries';
import type { Protocol, Resolver } from '../api/types';
import { PROTOCOL_LABEL, resolverNameError, upstreamUrlError } from '../lib/resolvers';
import { ConfirmDelete } from './ConfirmDelete';
import { EmptyState, Notice, type NoticeState } from './States';
import { Panel } from './Panel';

/** DoH or DoQ tag for a resolver. Renders nothing when the protocol is unknown. */
export function ProtocolBadge({ protocol }: { protocol: Protocol | null | undefined }) {
  if (!protocol) return null;
  return <span className="badge badge-cyan">{PROTOCOL_LABEL[protocol]}</span>;
}

/** Names as small badges, three at most, with a +N badge for the rest. */
export function UsedByBadges({ names, empty }: { names: string[]; empty: string }) {
  if (names.length === 0) return <span className="muted small">{empty}</span>;
  const shown = names.slice(0, 3);
  const extra = names.length - shown.length;
  return (
    <div className="chip-row">
      {shown.map((name) => (
        <span key={name} className="badge">{name}</span>
      ))}
      {extra > 0 && (
        <span className="badge" title={names.slice(3).join(', ')}>+{extra}</span>
      )}
    </div>
  );
}

/** Moves focus to the control marked data-focus-id once React has drawn it. */
export function focusControl(root: HTMLElement | null, id: string): void {
  requestAnimationFrame(() => {
    root?.querySelector<HTMLElement>(`[data-focus-id="${CSS.escape(id)}"]`)?.focus();
  });
}

export function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? '' : 's'}`;
}

interface FormProps {
  /** null adds a resolver. A resolver edits its name and URL. */
  editing: Resolver | null;
  onDone: (text: string) => void;
  onCancel: () => void;
}

function ResolverForm({ editing, onDone, onCancel }: FormProps) {
  const create = useCreateResolver();
  const update = useUpdateResolver();
  const [name, setName] = useState(editing?.name ?? '');
  const [url, setUrl] = useState(editing?.url ?? '');
  const [error, setError] = useState<string | null>(null);
  const busy = create.isPending || update.isPending;

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const body = { name: name.trim(), url: url.trim() };
    const problem = resolverNameError(body.name) ?? upstreamUrlError(body.url);
    if (problem) return setError(problem);
    setError(null);

    if (editing) {
      update.mutate(
        { name: editing.name, body },
        {
          onSuccess: () => onDone(`Saved resolver ${body.name}.`),
          onError: (err) => setError(err.message),
        },
      );
    } else {
      create.mutate(body, {
        onSuccess: () => onDone(`Added resolver ${body.name}.`),
        onError: (err) => setError(err.message),
      });
    }
  };

  const groups = editing?.groups ?? [];

  return (
    <form className="group-form" onSubmit={onSubmit} noValidate aria-label={editing ? `Edit ${editing.name}` : 'Add resolver'}>
      <div className="form-grid resolver-grid">
        <label className="field">
          <span>Friendly name</span>
          <input
            value={name}
            autoFocus
            placeholder="e.g. NextDNS (DoQ)"
            autoComplete="off"
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="field">
          <span>Server URL</span>
          <input
            className="mono"
            value={url}
            spellCheck={false}
            autoComplete="off"
            placeholder="quic://abc123.dns.nextdns.io or https://dns.example.net/dns-query"
            onChange={(e) => setUrl(e.target.value)}
          />
        </label>
      </div>
      {editing && (
        <p className="muted small">
          {groups.length > 0
            ? `Renaming updates ${plural(groups.length, 'group')} (${groups.join(', ')}). `
            : ''}
          Changing the URL starts its stats over.
        </p>
      )}
      {error && <Notice tone="error">{error}</Notice>}
      <div className="row-actions">
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? 'Saving...' : editing ? 'Save resolver' : 'Add resolver'}
        </button>
        <button type="button" className="btn" onClick={onCancel} disabled={busy}>
          Cancel
        </button>
      </div>
    </form>
  );
}

interface Props {
  resolvers: Resolver[];
  /** Another panel can raise this number to ask for the Add form to open. */
  openSignal?: number;
}

/** Admin panel: add, edit and delete the named upstream servers that groups use. */
export function ResolversPanel({ resolvers, openSignal = 0 }: Props) {
  const remove = useDeleteResolver();
  const rootRef = useRef<HTMLDivElement>(null);
  const [editing, setEditing] = useState<Resolver | 'new' | null>(null);
  const [confirmName, setConfirmName] = useState<string | null>(null);
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const deletingName = remove.isPending ? (remove.variables ?? null) : null;

  useEffect(() => {
    if (openSignal === 0) return;
    setEditing('new');
    rootRef.current?.scrollIntoView({ block: 'start' });
  }, [openSignal]);

  const closeForm = (focusId: string) => {
    setEditing(null);
    focusControl(rootRef.current, focusId);
  };

  const onDelete = (resolver: Resolver) => {
    setNotice(null);
    remove.mutate(resolver.name, {
      onSuccess: () => {
        setConfirmName(null);
        setNotice({ tone: 'ok', text: `Deleted resolver ${resolver.name}.` });
        focusControl(rootRef.current, 'add-resolver');
      },
      onError: (err) => {
        setConfirmName(null);
        setNotice({ tone: 'error', text: err.message });
      },
    });
  };

  return (
    <div ref={rootRef}>
      <Panel
        title={`Resolvers (${resolvers.length})`}
        actions={
          editing === null ? (
            <button
              type="button"
              data-focus-id="add-resolver"
              className="btn btn-small btn-primary"
              onClick={() => setEditing('new')}
            >
              Add resolver
            </button>
          ) : null
        }
      >
        <p className="muted small panel-sub">
          One DNS server each. Add the DoH and DoQ versions of a provider separately, and each gets its own dashboard card.
        </p>

        <div aria-live="polite" aria-atomic="true">
          {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}
        </div>

        {editing !== null && (
          <ResolverForm
            editing={editing === 'new' ? null : editing}
            onDone={(text) => {
              const focusId = editing === 'new' ? 'add-resolver' : `edit:${editing.name}`;
              setNotice({ tone: 'ok', text });
              closeForm(focusId);
            }}
            onCancel={() => closeForm(editing === 'new' ? 'add-resolver' : `edit:${editing.name}`)}
          />
        )}

        <div className="table-wrap stack">
          <table aria-label="Resolvers">
            <thead>
              <tr>
                <th scope="col">Resolver</th>
                <th scope="col">Protocol</th>
                <th scope="col">Server</th>
                <th scope="col">Used by</th>
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {resolvers.length === 0 && (
                <tr>
                  <td colSpan={5}>
                    <EmptyState>
                      No resolvers yet. Groups need at least one.{' '}
                      <button type="button" className="btn btn-small btn-primary" onClick={() => setEditing('new')}>
                        Add resolver
                      </button>
                    </EmptyState>
                  </td>
                </tr>
              )}
              {resolvers.map((resolver) => {
                const inUse = resolver.groups.length > 0;
                const reasonId = `resolver-reason-${resolver.name}`;
                return (
                  <tr key={resolver.name}>
                    <td className="col-name" data-label="Resolver">
                      <strong>{resolver.name}</strong>
                    </td>
                    <td className="col-protocol" data-label="Protocol">
                      <ProtocolBadge protocol={resolver.protocol} />
                    </td>
                    <td className="mono url-cell col-server" data-label="Server">
                      <div title={resolver.url}>{resolver.url}</div>
                    </td>
                    <td className="col-usedby" data-label="Used by">
                      <UsedByBadges names={resolver.groups} empty="No group" />
                    </td>
                    <td className="col-actions" data-label="Actions">
                      {confirmName === resolver.name ? (
                        <ConfirmDelete
                          prompt="Delete this resolver?"
                          deleting={deletingName === resolver.name}
                          disabled={remove.isPending}
                          onConfirm={() => onDelete(resolver)}
                          onCancel={() => setConfirmName(null)}
                        />
                      ) : (
                        <div className="row-actions">
                          <button
                            type="button"
                            data-focus-id={`edit:${resolver.name}`}
                            className="btn btn-small"
                            aria-label={`Edit ${resolver.name}`}
                            onClick={() => setEditing(resolver)}
                            disabled={editing !== null}
                          >
                            Edit
                          </button>
                          <button
                            type="button"
                            className="btn btn-small btn-danger-outline"
                            aria-label={`Delete ${resolver.name}`}
                            aria-describedby={inUse ? reasonId : undefined}
                            disabled={inUse}
                            onClick={() => setConfirmName(resolver.name)}
                          >
                            Delete
                          </button>
                          {inUse && (
                            <span id={reasonId} className="small muted">
                              In use by {plural(resolver.groups.length, 'group')}
                            </span>
                          )}
                        </div>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </Panel>
    </div>
  );
}
