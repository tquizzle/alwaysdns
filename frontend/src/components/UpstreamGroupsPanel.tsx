import { useEffect, useRef, useState, type FormEvent } from 'react';
import { useCreateGroup, useDeleteGroup, useUpdateGroup } from '../api/queries';
import type { Resolver, UpstreamGroup } from '../api/types';
import { PROTOCOL_LABEL, moveItem } from '../lib/resolvers';
import { ConfirmDelete } from './ConfirmDelete';
import { EmptyState, Notice, type NoticeState } from './States';
import { Panel } from './Panel';
import { ProtocolBadge, UsedByBadges, focusControl, plural } from './ResolversPanel';

const GROUP_NAME_MAX = 64;

interface PickerProps {
  /** Every resolver that exists. */
  resolvers: Resolver[];
  /** The chosen resolver names, in failover order. */
  selected: string[];
  onChange: (next: string[]) => void;
  /** Opens the Add resolver form on the same page. */
  onNewResolver?: () => void;
}

/** Ordered list of the chosen resolvers, with a dropdown and an Add button for the rest. */
function ResolverPicker({ resolvers, selected, onChange, onNewResolver }: PickerProps) {
  const [toAdd, setToAdd] = useState('');
  const [announce, setAnnounce] = useState('');
  const byName = new Map(resolvers.map((r) => [r.name, r]));
  const available = resolvers.filter((r) => !selected.includes(r.name));

  const move = (index: number, delta: -1 | 1) => {
    onChange(moveItem(selected, index, delta));
    setAnnounce(`Moved ${selected[index]} to position ${index + delta + 1}.`);
  };
  const remove = (name: string) => {
    onChange(selected.filter((n) => n !== name));
    setAnnounce(`Removed ${name}.`);
  };
  const add = () => {
    if (!toAdd) return;
    onChange([...selected, toAdd]);
    setAnnounce(`Added ${toAdd} at position ${selected.length + 1}.`);
    setToAdd('');
  };

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
              <span className={index === 0 ? 'badge badge-cyan' : 'badge'}>{index === 0 ? 'Primary' : 'Fallback'}</span>
              <span className="row-actions">
                <button
                  type="button"
                  className="btn btn-small btn-icon"
                  aria-label={`Move ${name} up`}
                  title="Move up"
                  disabled={index === 0}
                  onClick={() => move(index, -1)}
                >
                  <span aria-hidden="true">↑</span>
                </button>
                <button
                  type="button"
                  className="btn btn-small btn-icon"
                  aria-label={`Move ${name} down`}
                  title="Move down"
                  disabled={index === selected.length - 1}
                  onClick={() => move(index, 1)}
                >
                  <span aria-hidden="true">↓</span>
                </button>
                <button
                  type="button"
                  className="btn btn-small btn-icon btn-danger-outline"
                  aria-label={`Remove ${name}`}
                  title="Remove"
                  onClick={() => remove(name)}
                >
                  <span aria-hidden="true">✕</span>
                </button>
              </span>
            </li>
          ))}
        </ol>
      )}
      {selected.length === 1 && (
        <p className="muted small">Only one resolver, so nothing takes over if it fails. Add a fallback below.</p>
      )}
      <p className="sr-only" aria-live="polite">{announce}</p>

      {resolvers.length === 0 ? (
        <p className="muted small">
          No resolvers exist yet.{' '}
          {onNewResolver && (
            <button type="button" className="btn btn-small" onClick={onNewResolver}>
              Add a resolver
            </button>
          )}
        </p>
      ) : (
        <div className="picker-add">
          <select
            aria-label="Resolver to add"
            value={toAdd}
            disabled={available.length === 0}
            onChange={(e) => setToAdd(e.target.value)}
          >
            <option value="">
              {available.length === 0 ? 'Every resolver is already in this group' : 'Choose a resolver...'}
            </option>
            {available.map((r) => (
              <option key={r.name} value={r.name}>
                {r.name}
                {r.protocol ? ` (${PROTOCOL_LABEL[r.protocol]})` : ''}
              </option>
            ))}
          </select>
          <button type="button" className="btn btn-small" disabled={!toAdd} onClick={add}>
            Add to group
          </button>
          {onNewResolver && (
            <button type="button" className="btn btn-small" onClick={onNewResolver}>
              New resolver…
            </button>
          )}
        </div>
      )}
    </div>
  );
}

interface FormProps {
  /** null creates a group. A group edits its resolvers only, since its name is its key. */
  editing: UpstreamGroup | null;
  resolvers: Resolver[];
  onDone: (text: string) => void;
  onCancel: () => void;
  onNewResolver?: () => void;
}

function GroupForm({ editing, resolvers, onDone, onCancel, onNewResolver }: FormProps) {
  const create = useCreateGroup();
  const update = useUpdateGroup();
  const formRef = useRef<HTMLFormElement>(null);
  const [name, setName] = useState(editing?.name ?? '');
  const [selected, setSelected] = useState<string[]>(editing?.resolvers ?? []);
  const [error, setError] = useState<string | null>(null);
  const busy = create.isPending || update.isPending;

  // Editing has no name field to focus, so the form itself takes focus and is announced by its label.
  useEffect(() => {
    if (editing) formRef.current?.focus();
  }, [editing]);

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

  const affected = editing?.clients ?? [];

  return (
    <form
      ref={formRef}
      tabIndex={-1}
      className="group-form"
      onSubmit={onSubmit}
      noValidate
      aria-label={editing ? `Edit ${editing.name}` : 'Add upstream group'}
    >
      <div className="form-grid group-grid">
        <label className="field">
          <span>Name</span>
          <input
            value={name}
            autoFocus={editing === null}
            placeholder="e.g. Family DoQ"
            autoComplete="off"
            disabled={editing !== null}
            onChange={(e) => setName(e.target.value)}
          />
          {editing && <span className="muted small">A group&apos;s name is its key. Create a new group to use another name.</span>}
        </label>
        <ResolverPicker resolvers={resolvers} selected={selected} onChange={setSelected} onNewResolver={onNewResolver} />
      </div>
      {affected.length > 0 && (
        <p className="muted small">Changes apply to {plural(affected.length, 'client')}: {affected.join(', ')}.</p>
      )}
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

function deleteBlocker(group: UpstreamGroup): string | null {
  if (group.is_default) return 'Default group';
  if (group.clients.length > 0) return `In use by ${plural(group.clients.length, 'client')}`;
  return null;
}

interface Props {
  groups: UpstreamGroup[];
  resolvers: Resolver[];
  /** Opens the Add resolver form, for groups that need a resolver that does not exist yet. */
  onNewResolver?: () => void;
}

/** Admin panel: add, edit and delete upstream groups. */
export function UpstreamGroupsPanel({ groups, resolvers, onNewResolver }: Props) {
  const remove = useDeleteGroup();
  const rootRef = useRef<HTMLDivElement>(null);
  const [editing, setEditing] = useState<UpstreamGroup | 'new' | null>(null);
  const [confirmName, setConfirmName] = useState<string | null>(null);
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const byName = new Map(resolvers.map((r) => [r.name, r]));
  const deletingName = remove.isPending ? (remove.variables ?? null) : null;

  const closeForm = (focusId: string) => {
    setEditing(null);
    focusControl(rootRef.current, focusId);
  };

  const onDelete = (group: UpstreamGroup) => {
    setNotice(null);
    remove.mutate(group.name, {
      onSuccess: () => {
        setConfirmName(null);
        setNotice({ tone: 'ok', text: `Deleted upstream group ${group.name}.` });
        focusControl(rootRef.current, 'add-group');
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
        title={`Upstream groups (${groups.length})`}
        actions={
          editing === null ? (
            <button
              type="button"
              data-focus-id="add-group"
              className="btn btn-small btn-primary"
              onClick={() => setEditing('new')}
            >
              Add group
            </button>
          ) : null
        }
      >
        <p className="muted small panel-sub">
          An ordered list of resolvers. The first is tried first, and the next take over if it fails. Clients with no group use the default.
        </p>

        <div aria-live="polite" aria-atomic="true">
          {notice && <Notice tone={notice.tone}>{notice.text}</Notice>}
        </div>

        {editing !== null && (
          <GroupForm
            editing={editing === 'new' ? null : editing}
            resolvers={resolvers}
            onNewResolver={onNewResolver}
            onDone={(text) => {
              const focusId = editing === 'new' ? 'add-group' : `edit:${editing.name}`;
              setNotice({ tone: 'ok', text });
              closeForm(focusId);
            }}
            onCancel={() => closeForm(editing === 'new' ? 'add-group' : `edit:${editing.name}`)}
          />
        )}

        <div className="table-wrap stack">
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
                  <td colSpan={4}>
                    <EmptyState>
                      No upstream groups yet.{' '}
                      <button type="button" className="btn btn-small btn-primary" onClick={() => setEditing('new')}>
                        Add group
                      </button>
                    </EmptyState>
                  </td>
                </tr>
              )}
              {groups.map((group) => {
                const blocker = deleteBlocker(group);
                const reasonId = `group-reason-${group.name}`;
                return (
                  <tr key={group.name}>
                    <td className="col-name" data-label="Group">
                      <strong>{group.name}</strong>
                      {group.is_default && <span className="badge badge-cyan group-default">Default</span>}
                    </td>
                    <td className="url-cell col-resolvers" data-label="Resolvers">
                      {group.resolvers.map((name, index) => (
                        <div key={name} title={byName.get(name)?.url ?? name}>
                          <span className="muted mono">{index + 1}.</span> {name}{' '}
                          <ProtocolBadge protocol={byName.get(name)?.protocol} />
                        </div>
                      ))}
                    </td>
                    <td className="col-usedby" data-label="Used by">
                      <UsedByBadges names={group.clients} empty={group.is_default ? 'Clients with no group' : 'Nothing'} />
                    </td>
                    <td className="col-actions" data-label="Actions">
                      {confirmName === group.name ? (
                        <ConfirmDelete
                          prompt="Delete this group?"
                          deleting={deletingName === group.name}
                          disabled={remove.isPending}
                          onConfirm={() => onDelete(group)}
                          onCancel={() => setConfirmName(null)}
                        />
                      ) : (
                        <div className="row-actions">
                          <button
                            type="button"
                            data-focus-id={`edit:${group.name}`}
                            className="btn btn-small"
                            aria-label={`Edit ${group.name}`}
                            onClick={() => setEditing(group)}
                            disabled={editing !== null}
                          >
                            Edit
                          </button>
                          <button
                            type="button"
                            className="btn btn-small btn-danger-outline"
                            aria-label={`Delete ${group.name}`}
                            aria-describedby={blocker ? reasonId : undefined}
                            disabled={blocker !== null}
                            onClick={() => setConfirmName(group.name)}
                          >
                            Delete
                          </button>
                          {blocker && (
                            <span id={reasonId} className="small muted">{blocker}</span>
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
