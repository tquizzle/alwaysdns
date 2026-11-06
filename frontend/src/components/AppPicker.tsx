import { useState } from 'react';
import { useServices } from '../api/queries';
import { groupServices } from '../lib/services';

interface Props {
  /** Catalogue ids this client blocks. */
  selected: string[];
  onToggle: (id: string) => void;
}

/** Searchable, grouped list of apps a client can block, such as YouTube or Discord. */
export function AppPicker({ selected, onToggle }: Props) {
  const services = useServices();
  const [query, setQuery] = useState('');

  const items = services.data?.services ?? [];
  const groups = groupServices(items, query);
  // Ids the client keeps but the catalogue no longer lists. They stay removable.
  const known = new Set(items.map((s) => s.id));
  const missing = services.data ? selected.filter((id) => !known.has(id)) : [];

  return (
    <div className="field">
      <span id="apps-label">Blocked apps{selected.length > 0 ? ` (${selected.length})` : ''}</span>
      {services.isPending && <span className="muted small">Loading apps...</span>}
      {services.isError && <span className="muted small">Could not load the app list.</span>}
      {services.data && items.length === 0 && (
        <span className="muted small">
          The app catalogue is not available yet. It downloads at start-up and once a day.
        </span>
      )}
      {services.data?.note && <span className="muted small">Using the saved copy of the app catalogue.</span>}

      {items.length > 0 && (
        <>
          <input
            type="search"
            value={query}
            placeholder="Search apps, e.g. YouTube"
            aria-label="Search apps"
            autoComplete="off"
            onChange={(e) => setQuery(e.target.value)}
          />
          <div className="apps-box" role="group" aria-labelledby="apps-label">
            {groups.length === 0 && <span className="muted small">No app matches.</span>}
            {groups.map((g) => (
              <div key={g.group} className="apps-group">
                <span className="muted small">{g.label}</span>
                <div className="chip-row">
                  {g.services.map((s) => (
                    <button
                      key={s.id}
                      type="button"
                      className={`chip ${selected.includes(s.id) ? 'is-on' : ''}`}
                      aria-pressed={selected.includes(s.id)}
                      title={`${s.domains} domains`}
                      onClick={() => onToggle(s.id)}
                    >
                      {s.name}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </>
      )}

      {missing.length > 0 && (
        <div className="apps-group">
          <span className="muted small">No longer in the catalogue</span>
          <div className="chip-row">
            {missing.map((id) => (
              <button key={id} type="button" className="chip is-on" aria-pressed="true" onClick={() => onToggle(id)}>
                {id}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
