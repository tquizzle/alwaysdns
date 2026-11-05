import { useState, type FormEvent } from 'react';
import { useCreateClient, useUpdateClient } from '../api/queries';
import type { Client, ClientUpdateBody } from '../api/types';
import { cidrError, linesToList } from '../lib/clients';
import { Modal } from './Modal';
import { Notice } from './States';

interface Props {
  /** null adds a client. A client edits its rules. Its name is its key, so it cannot change. */
  client: Client | null;
  groupNames: string[];
  defaultGroup: string;
  blockListNames: string[];
  onClose: () => void;
  onSaved: (text: string) => void;
}

const NAME_MAX = 64;

/** Admin dialog that adds a configured client or edits its networks, groups, lists and domains. */
export function ClientDialog({ client, groupNames, defaultGroup, blockListNames, onClose, onSaved }: Props) {
  const create = useCreateClient();
  const update = useUpdateClient();
  const busy = create.isPending || update.isPending;

  const [name, setName] = useState(client?.name ?? '');
  const [cidrs, setCidrs] = useState((client?.cidrs ?? []).join('\n'));
  const [group, setGroup] = useState(client?.upstream_group ?? defaultGroup);
  const [lists, setLists] = useState<string[]>(client?.block_lists ?? []);
  const [allow, setAllow] = useState((client?.allow_domains ?? []).join('\n'));
  const [block, setBlock] = useState((client?.block_domains ?? []).join('\n'));
  const [error, setError] = useState<string | null>(null);

  const groups = groupNames.includes(group) ? groupNames : [group, ...groupNames];

  const toggleList = (list: string) => {
    setLists((prev) => (prev.includes(list) ? prev.filter((l) => l !== list) : [...prev, list]));
  };

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const trimmedName = name.trim();
    if (!client) {
      if (trimmedName.length === 0) return setError('Give the client a name.');
      if (trimmedName.length > NAME_MAX) return setError(`The name must be ${NAME_MAX} characters or fewer.`);
    }
    const networks = linesToList(cidrs);
    for (const network of networks) {
      const problem = cidrError(network);
      if (problem) return setError(problem);
    }
    setError(null);

    const body: ClientUpdateBody = {
      cidrs: networks,
      upstream_group: group,
      block_lists: lists,
      allow_domains: linesToList(allow),
      block_domains: linesToList(block),
    };

    if (client) {
      update.mutate(
        { name: client.name, body },
        {
          onSuccess: () => onSaved(`Saved ${client.name}.`),
          onError: (err) => setError(err.message),
        },
      );
    } else {
      create.mutate(
        { ...body, name: trimmedName },
        {
          onSuccess: () => onSaved(`Added client ${trimmedName}.`),
          onError: (err) => setError(err.message),
        },
      );
    }
  };

  return (
    <Modal title={client ? `Edit ${client.name}` : 'Add a client'} onClose={onClose} wide>
      <form className="client-form" onSubmit={onSubmit} noValidate>
        <div className="form-grid">
          <label className="field">
            <span>Name</span>
            <input
              value={name}
              placeholder="e.g. Living room TV"
              autoComplete="off"
              disabled={client !== null}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <div className="field">
            <span id="client-group-label">Upstream group</span>
            <select aria-labelledby="client-group-label" value={group} onChange={(e) => setGroup(e.target.value)}>
              {groups.map((g) => (
                <option key={g} value={g}>{g}</option>
              ))}
            </select>
            <a className="small" href="#/upstreams" onClick={onClose}>
              Manage upstream groups
            </a>
          </div>
          <label className="field wide">
            <span>Networks, one per line (CIDR)</span>
            <textarea
              className="mono"
              rows={2}
              value={cidrs}
              spellCheck={false}
              placeholder={'192.168.1.10/32\n198.51.100.0/24'}
              onChange={(e) => setCidrs(e.target.value)}
            />
          </label>
        </div>

        <div className="field">
          <span>Block lists</span>
          {blockListNames.length === 0 ? (
            <span className="muted small">No block lists yet. Add one on the Lists page.</span>
          ) : (
            <div className="chip-row" role="group" aria-label="Block lists for this client">
              {blockListNames.map((name) => (
                <button
                  key={name}
                  type="button"
                  className={`chip ${lists.includes(name) ? 'is-on' : ''}`}
                  aria-pressed={lists.includes(name)}
                  onClick={() => toggleList(name)}
                >
                  {name}
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="form-grid">
          <label className="field">
            <span>Allow these domains, one per line</span>
            <textarea className="mono" rows={4} value={allow} spellCheck={false} onChange={(e) => setAllow(e.target.value)} />
          </label>
          <label className="field">
            <span>Block these domains, one per line</span>
            <textarea className="mono" rows={4} value={block} spellCheck={false} onChange={(e) => setBlock(e.target.value)} />
          </label>
        </div>

        {error && <Notice tone="error">{error}</Notice>}

        <div className="row-actions">
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? 'Saving...' : client ? 'Save client' : 'Add client'}
          </button>
          <button type="button" className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
        </div>
      </form>
    </Modal>
  );
}
