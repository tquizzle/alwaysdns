import { useState, type FormEvent } from 'react';
import { useCreateGroup, useDeleteGroup, useUpdateGroup } from '../api/queries';
import type { Resolver, UpstreamGroup } from '../api/types';
import { PROTOCOL_LABEL, moveItem } from '../lib/resolvers';
import { Notice, type NoticeState } from './States';
import { Panel } from './Panel';
import { ProtocolBadge } from './ResolversPanel';

const GROUP_NAME_MAX = 64;

interface PickerProps {
  /** Every resolver that exists. */
  resolvers: Resolver[];
  /** The chosen resolver names, in failover order. */
  selected: string[];
  onChange: (next: string[]) => void;
}

/** Ordered list of the chosen resolvers, with a dropdown to add the rest. */
function ResolverPicker({ resolvers, selected, onChange }: PickerProps) {
  const byName = new Map(resolvers.map((r) => [r.name, r]));
  const available = resolvers.filter((r) => !selected.includes(r.name));

  return (
    <div className="field wide">
      <span>Resolvers, tried in order</span>
      {selected.length === 0 ? (
        <p className="muted small">None chosen yet.</p>
      ) : (
        <ol className="picker-list">
          {selected.map((name, index) => (
            <li key={name} className="picker-item">
              <span className="picker-index mono">{index + 1}</span>
              <span className="picker-name" title={byName.get(name)?.url ?? name}>{name}</span>
              <ProtocolBadge protocol={byName.get(name)?.protocol} />
              <span className="row-actions">
                <button
                  type="button"
                  className="btn btn-small"
                  aria-label={`Move ${name} up`}
                  disabled={index === 0}
                  onClick={() => onChange(moveItem(selected, index, -1))}
                >
                  Up
                </button>
                <button
                  type="button"
                  className="btn btn-small"
                  aria-label={`Move ${name} down`}
                  disabled={index === selected.length - 1}
                  onClick={() => onChange(moveItem(selected, index, 1))}
                >
                  Down
                </button>
                <button
                  type="button"
                  className="btn btn-small btn-danger-outline"
                  aria-label={`Remove ${name}`}
                  onClick={() => onChange(selected.filter((n) => n !== name))}
                >
                  Remove
                </button>
              </span>
            </li>
          ))}
        </ol>
      )}
      <select
        aria-label="Add a resolver"
        value=""
        disabled={available.length === 0}
        onChange={(e) => {
          if (e.target.value) onChange([...selected, e.target.value]);
        }}
      >
        <option value="">
          {resolvers.length === 0
            ? 'Add a resolver first'
            : available.length === 0
              ? 'Every resolver is already in this group'
              : 'Add a resolver...'}
        </option>
        {available.map((r) => (
          <option key={r.name} value={r.name}>
            {r.name}
            {r.protocol ? ` (${PROTOCOL_LABEL[r.protocol]})` : ''}
          </option>
        ))}
      </select>
    </div>
  );
}

interface FormProps {
  /** null creates a group. A group edits its resolvers only, since its name is its key. */
  editing: UpstreamGroup | null;
  resolvers: Resolver[];
  onDone: (text: string) => void;
  onCancel: () => void;
}

function GroupForm({ editing, resolvers, onDone, onCancel }: FormProps) {
  const create = useCreateGroup();
  const update = useUpdateGroup();
  const [name, setName] = useState(editing?.name ?? '');
  const [selected, setSelected] = useState<string[]>(editing?.resolvers ?? []);
  const [error, setError] = useState<string | null>(null);
  const busy = create.isPending || update.isPending;

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const trimmedName = name.trim();

    if (!editing && trimmedName.length === 0) return setError('Give the group a name.');
    if (trimmedName.length > GROUP_NAME_MAX) return setError(`The name must be ${GROUP_NAME_MAX} characters or fewer.`);
    if (selected.length === 0) return setError('Add at least one resolver.');
    setError(null);

    if (editing) {
      update.mutate(
        { name: editing.name, body: { resolvers: selected } },
        {
          onSuccess: () => onDone(`Updated upstream group ${editing.name}.`),
          onError: (err) => setError(err.message),
        },
      );
    } else {
      create.mutate(
        { name: trimmedName, resolvers: selected },
        {
          onSuccess: () => onDone(`Added upstream group ${trimmedName}.`),
          onError: (err) => setError(err.message),
        },
      );
    }
  };

  return (
    <form className="group-form" onSubmit={onSubmit} noValidate aria-label={editing ? `Edit ${editing.name}` : 'Add upstream group'}>
      <div className="form-grid group-grid">
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
        <ResolverPicker resolvers={resolvers} selected={selected} onChange={setSelected} />
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
  resolvers: Resolver[];
}

/** Admin panel: add, edit and delete upstream groups. */
export function UpstreamGroupsPanel({ groups, resolvers }: Props) {
  const remove = useDeleteGroup();
  const [editing, setEditing] = useState<UpstreamGroup | 'new' | null>(null);
  const [confirmName, setConfirmName] = useState<string | null>(null);
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const byName = new Map(resolvers.map((r) => [r.name, r]));

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
        A group is an ordered list of resolvers a client resolves through. The first is tried first, and the next ones take
        over if it fails. Clients with no group use the default.
      </p>

      {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}

      {editing !== null && (
        <GroupForm
          editing={editing === 'new' ? null : editing}
          resolvers={resolvers}
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
              <th scope="col">Resolvers</th>
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
                <td className="url-cell">
                  {group.resolvers.map((name, index) => (
                    <div key={name} title={byName.get(name)?.url ?? name}>
                      <span className="muted mono">{index + 1}.</span> {name} <ProtocolBadge protocol={byName.get(name)?.protocol} />
                    </div>
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
