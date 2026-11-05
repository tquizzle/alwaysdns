import { useState, type FormEvent } from 'react';
import { useCreateResolver, useDeleteResolver, useUpdateResolver } from '../api/queries';
import type { Protocol, Resolver } from '../api/types';
import { PROTOCOL_LABEL, resolverNameError, upstreamUrlError } from '../lib/resolvers';
import { Notice, type NoticeState } from './States';
import { Panel } from './Panel';

/** DoH or DoQ tag for a resolver. Renders nothing when the protocol is unknown. */
export function ProtocolBadge({ protocol }: { protocol: Protocol | null | undefined }) {
  if (!protocol) return null;
  return <span className="badge badge-cyan">{PROTOCOL_LABEL[protocol]}</span>;
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

  return (
    <form className="group-form" onSubmit={onSubmit} noValidate aria-label={editing ? `Edit ${editing.name}` : 'Add resolver'}>
      <div className="form-grid resolver-grid">
        <label className="field">
          <span>Friendly name</span>
          <input
            value={name}
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
        <p className="muted small">Renaming updates every group that uses this resolver. Changing the URL starts its stats over.</p>
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
}

/** Admin panel: add, edit and delete the named upstream servers that groups use. */
export function ResolversPanel({ resolvers }: Props) {
  const remove = useDeleteResolver();
  const [editing, setEditing] = useState<Resolver | 'new' | null>(null);
  const [confirmName, setConfirmName] = useState<string | null>(null);
  const [notice, setNotice] = useState<NoticeState | null>(null);

  const onDelete = (resolver: Resolver) => {
    setNotice(null);
    remove.mutate(resolver.name, {
      onSuccess: () => {
        setConfirmName(null);
        setNotice({ tone: 'ok', text: `Deleted resolver ${resolver.name}.` });
      },
      onError: (err) => {
        setConfirmName(null);
        setNotice({ tone: 'error', text: err.message });
      },
    });
  };

  return (
    <Panel
      title="Resolvers"
      actions={
        editing === null ? (
          <button type="button" className="btn btn-small btn-primary" onClick={() => setEditing('new')}>
            Add resolver
          </button>
        ) : null
      }
    >
      <p className="muted small panel-sub">
        A resolver is one DNS server with a friendly name. Each one gets its own stats card on the dashboard, so add the DoH
        and DoQ versions of a provider separately. Groups below choose which resolvers a client uses.
      </p>

      {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}

      {editing !== null && (
        <ResolverForm
          editing={editing === 'new' ? null : editing}
          onDone={(text) => {
            setEditing(null);
            setNotice({ tone: 'ok', text });
          }}
          onCancel={() => setEditing(null)}
        />
      )}

      <div className="table-wrap">
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
                <td colSpan={5} className="muted">No resolvers yet.</td>
              </tr>
            )}
            {resolvers.map((resolver) => (
              <tr key={resolver.name}>
                <td>
                  <strong>{resolver.name}</strong>
                </td>
                <td>
                  <ProtocolBadge protocol={resolver.protocol} />
                </td>
                <td className="mono url-cell">
                  <div title={resolver.url}>{resolver.url}</div>
                </td>
                <td className="muted small">
                  {resolver.groups.length === 0 ? 'No group' : resolver.groups.join(', ')}
                </td>
                <td>
                  {confirmName === resolver.name ? (
                    <div className="row-actions">
                      <span className="small text-red">Delete this resolver?</span>
                      <button
                        type="button"
                        className="btn btn-small btn-danger"
                        disabled={remove.isPending}
                        onClick={() => onDelete(resolver)}
                      >
                        {remove.isPending ? 'Deleting...' : 'Confirm'}
                      </button>
                      <button type="button" className="btn btn-small" onClick={() => setConfirmName(null)}>
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <div className="row-actions">
                      <button
                        type="button"
                        className="btn btn-small"
                        onClick={() => setEditing(resolver)}
                        disabled={editing !== null}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="btn btn-small btn-danger-outline"
                        disabled={resolver.groups.length > 0}
                        title={
                          resolver.groups.length > 0
                            ? `Remove it from ${resolver.groups.join(', ')} first`
                            : 'Delete this resolver'
                        }
                        onClick={() => setConfirmName(resolver.name)}
                      >
                        Delete
                      </button>
                    </div>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}
