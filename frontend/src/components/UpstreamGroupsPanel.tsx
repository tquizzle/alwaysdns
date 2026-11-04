import { useState, type FormEvent } from 'react';
import { useCreateGroup, useDeleteGroup, useUpdateGroup } from '../api/queries';
import type { UpstreamGroup } from '../api/types';
import { Notice, type NoticeState } from './States';
import { Panel } from './Panel';

const GROUP_NAME_MAX = 64;

/** Returns an error message, or null when the upstream URL is usable. */
function upstreamError(raw: string): string | null {
  const value = raw.trim();
  if (!value.startsWith('https://') && !value.startsWith('quic://')) {
    return `"${value}" must start with https:// (DoH) or quic:// (DoQ)`;
  }
  try {
    new URL(value);
  } catch {
    return `"${value}" is not a valid URL`;
  }
  return null;
}

interface FormProps {
  /** null creates a group. A group edits its servers only, since its name is its key. */
  editing: UpstreamGroup | null;
  onDone: (text: string) => void;
  onCancel: () => void;
}

function GroupForm({ editing, onDone, onCancel }: FormProps) {
  const create = useCreateGroup();
  const update = useUpdateGroup();
  const [name, setName] = useState(editing?.name ?? '');
  const [servers, setServers] = useState((editing?.upstreams ?? []).join('\n'));
  const [error, setError] = useState<string | null>(null);
  const busy = create.isPending || update.isPending;

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const trimmedName = name.trim();
    const urls = servers
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.length > 0);

    if (!editing && trimmedName.length === 0) return setError('Give the group a name.');
    if (trimmedName.length > GROUP_NAME_MAX) return setError(`The name must be ${GROUP_NAME_MAX} characters or fewer.`);
    if (urls.length === 0) return setError('Add at least one upstream server, one per line.');
    for (const url of urls) {
      const problem = upstreamError(url);
      if (problem) return setError(problem);
    }
    setError(null);

    if (editing) {
      update.mutate(
        { name: editing.name, body: { upstreams: urls } },
        {
          onSuccess: () => onDone(`Updated upstream group ${editing.name}.`),
          onError: (err) => setError(err.message),
        },
      );
    } else {
      create.mutate(
        { name: trimmedName, upstreams: urls },
        {
          onSuccess: () => onDone(`Added upstream group ${trimmedName}.`),
          onError: (err) => setError(err.message),
        },
      );
    }
  };

  return (
    <form className="group-form" onSubmit={onSubmit} noValidate aria-label={editing ? `Edit ${editing.name}` : 'Add upstream group'}>
      <div className="form-grid">
        <label className="field">
          <span>Name</span>
          <input
            value={name}
            placeholder="e.g. Family DoQ"
            autoComplete="off"
            disabled={editing !== null}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="field wide">
          <span>Upstream servers, one per line</span>
          <textarea
            className="mono"
            rows={3}
            value={servers}
            spellCheck={false}
            placeholder={'quic://abc123.dns.nextdns.io\nhttps://dns.example.net/dns-query'}
            onChange={(e) => setServers(e.target.value)}
          />
        </label>
      </div>
      {error && <Notice tone="error">{error}</Notice>}
      <div className="row-actions">
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? 'Saving...' : editing ? 'Save group' : 'Add group'}
        </button>
        <button type="button" className="btn" onClick={onCancel} disabled={busy}>
          Cancel
        </button>
      </div>
    </form>
  );
}

interface Props {
  groups: UpstreamGroup[];
}

/** Admin panel: add, edit and delete upstream groups. */
export function UpstreamGroupsPanel({ groups }: Props) {
  const remove = useDeleteGroup();
  const [editing, setEditing] = useState<UpstreamGroup | 'new' | null>(null);
  const [confirmName, setConfirmName] = useState<string | null>(null);
  const [notice, setNotice] = useState<NoticeState | null>(null);

  const onDelete = (group: UpstreamGroup) => {
    setNotice(null);
    remove.mutate(group.name, {
      onSuccess: () => {
        setConfirmName(null);
        setNotice({ tone: 'ok', text: `Deleted upstream group ${group.name}.` });
      },
      onError: (err) => {
        setConfirmName(null);
        setNotice({ tone: 'error', text: err.message });
      },
    });
  };

  return (
    <Panel
      title="Upstream groups"
      actions={
        editing === null ? (
          <button type="button" className="btn btn-small btn-primary" onClick={() => setEditing('new')}>
            Add group
          </button>
        ) : null
      }
    >
      <p className="muted small panel-sub">
        A group is the set of DNS servers a client resolves through. Clients with no group use the default.
      </p>

      {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}

      {editing !== null && (
        <GroupForm
          editing={editing === 'new' ? null : editing}
          onDone={(text) => {
            setEditing(null);
            setNotice({ tone: 'ok', text });
          }}
          onCancel={() => setEditing(null)}
        />
      )}

      <div className="table-wrap">
        <table aria-label="Upstream groups">
          <thead>
            <tr>
              <th scope="col">Group</th>
              <th scope="col">Servers</th>
              <th scope="col">Used by</th>
              <th scope="col">Actions</th>
            </tr>
          </thead>
          <tbody>
            {groups.length === 0 && (
              <tr>
                <td colSpan={4} className="muted">No upstream groups yet.</td>
              </tr>
            )}
            {groups.map((group) => (
              <tr key={group.name}>
                <td>
                  <strong>{group.name}</strong>
                  {group.is_default && <span className="badge badge-cyan group-default">Default</span>}
                </td>
                <td className="mono url-cell">
                  {group.upstreams.map((url) => (
                    <div key={url} title={url}>{url}</div>
                  ))}
                </td>
                <td className="muted small">
                  {group.clients.length === 0
                    ? group.is_default
                      ? 'Clients with no group'
                      : 'Nothing'
                    : group.clients.join(', ')}
                </td>
                <td>
                  {confirmName === group.name ? (
                    <div className="row-actions">
                      <span className="small text-red">Delete this group?</span>
                      <button
                        type="button"
                        className="btn btn-small btn-danger"
                        disabled={remove.isPending}
                        onClick={() => onDelete(group)}
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
                        onClick={() => setEditing(group)}
                        disabled={editing !== null}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="btn btn-small btn-danger-outline"
                        disabled={group.is_default || group.clients.length > 0}
                        title={
                          group.is_default
                            ? 'The default group cannot be deleted'
                            : group.clients.length > 0
                              ? 'Move these clients to another group first'
                              : 'Delete this group'
                        }
                        onClick={() => setConfirmName(group.name)}
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
