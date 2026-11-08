import { useState, type FormEvent } from 'react';
import { api } from '../api/endpoints';
import { useUpdateSettings } from '../api/queries';
import type { Rewrite, Settings, SettingsBody } from '../api/types';
import { linesToList } from '../lib/clients';
import { exportFilename, nonBlankRows, parseDays, rewriteError, settingsBody } from '../lib/settings';
import { Panel } from './Panel';
import { Notice } from './States';

interface Message {
  tone: 'ok' | 'error';
  text: string;
}

/** Saves one section. Every other section is sent as the server has it, so it cannot be cleared. */
function useSectionSave(settings: Settings) {
  const update = useUpdateSettings();
  const [message, setMessage] = useState<Message | null>(null);
  const save = (patch: Partial<SettingsBody>, okText: string) => {
    setMessage(null);
    update.mutate(settingsBody(settings, patch), {
      onSuccess: () => setMessage({ tone: 'ok', text: okText }),
      onError: (err) => setMessage({ tone: 'error', text: err.message }),
    });
  };
  return { save, message, setMessage, busy: update.isPending };
}

function SaveRow({ busy, message }: { busy: boolean; message: Message | null }) {
  return (
    <>
      {message && <Notice tone={message.tone}>{message.text}</Notice>}
      <div className="row-actions">
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? 'Saving...' : 'Save'}
        </button>
      </div>
    </>
  );
}

/** Names that are answered with a fixed address instead of asking an upstream. */
export function LocalAnswersPanel({ settings }: { settings: Settings }) {
  const { save, message, setMessage, busy } = useSectionSave(settings);
  const [rows, setRows] = useState<Rewrite[]>(settings.rewrites);

  const change = (index: number, patch: Partial<Rewrite>) =>
    setRows((prev) => prev.map((row, i) => (i === index ? { ...row, ...patch } : row)));

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const filled = nonBlankRows(rows);
    for (const row of filled) {
      const problem = rewriteError(row);
      if (problem) return setMessage({ tone: 'error', text: problem });
    }
    setRows(filled);
    save({ rewrites: filled }, 'Saved local answers.');
  };

  return (
    <Panel
      title="Local answers"
      subtitle="Answer a name with a fixed address, without asking an upstream. Use *.example.test to cover every name under it. An exact name wins over a wildcard."
    >
      <form className="settings-form" onSubmit={onSubmit} noValidate>
        {rows.length === 0 && <p className="muted small">No local answers yet.</p>}
        {rows.map((row, i) => (
          <div className="rewrite-row" key={i}>
            <input
              className="mono"
              value={row.domain}
              placeholder="nas.example.test or *.example.test"
              aria-label={`Name for local answer ${i + 1}`}
              spellCheck={false}
              onChange={(e) => change(i, { domain: e.target.value })}
            />
            <input
              className="mono"
              value={row.answer}
              placeholder="192.0.2.5"
              aria-label={`IP address for local answer ${i + 1}`}
              spellCheck={false}
              onChange={(e) => change(i, { answer: e.target.value })}
            />
            <button
              type="button"
              className="btn btn-small"
              aria-label={`Remove local answer ${i + 1}`}
              onClick={() => setRows((prev) => prev.filter((_, j) => j !== i))}
            >
              Remove
            </button>
          </div>
        ))}
        <div className="row-actions">
          <button type="button" className="btn" onClick={() => setRows((prev) => [...prev, { domain: '', answer: '' }])}>
            Add a local answer
          </button>
        </div>
        <SaveRow busy={busy} message={message} />
      </form>
    </Panel>
  );
}

const DOMAIN_COPY = {
  allow_domains: {
    title: 'Global allow domains',
    subtitle: 'Never blocked, for every device. Also covers subdomains, and overrides every block list.',
  },
  block_domains: {
    title: 'Global block domains',
    subtitle: 'Always blocked, for every device. Also covers subdomains.',
  },
} as const;

/** One global domain list, edited as one name per line. */
export function DomainsPanel({ settings, field }: { settings: Settings; field: 'allow_domains' | 'block_domains' }) {
  const { save, message, busy } = useSectionSave(settings);
  const [text, setText] = useState(settings[field].join('\n'));
  const copy = DOMAIN_COPY[field];

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    save({ [field]: linesToList(text) }, `Saved ${copy.title.toLowerCase()}.`);
  };

  return (
    <Panel title={copy.title} subtitle={copy.subtitle}>
      <form className="settings-form" onSubmit={onSubmit} noValidate>
        <label className="field">
          <span className="sr-only">{copy.title}, one per line</span>
          <textarea
            className="mono"
            rows={6}
            value={text}
            spellCheck={false}
            placeholder={'example.com\nads.example.net'}
            onChange={(e) => setText(e.target.value)}
          />
        </label>
        <SaveRow busy={busy} message={message} />
      </form>
    </Panel>
  );
}

/** The plain addresses used to find DoH and DoQ hosts, and the group clients use by default. */
export function ResolvingPanel({ settings }: { settings: Settings }) {
  const { save, message, busy } = useSectionSave(settings);
  const [bootstrap, setBootstrap] = useState(settings.bootstrap.join('\n'));
  const [group, setGroup] = useState(settings.default_upstream_group);

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    save({ bootstrap: linesToList(bootstrap), default_upstream_group: group }, 'Saved resolver defaults.');
  };

  return (
    <Panel
      title="Resolver defaults"
      subtitle="Bootstrap addresses find the servers behind named resolvers. The default group is used by devices that match no client."
    >
      <form className="settings-form" onSubmit={onSubmit} noValidate>
        <div className="form-grid">
          <label className="field">
            <span>Bootstrap resolvers, one IP per line</span>
            <textarea
              className="mono"
              rows={3}
              value={bootstrap}
              spellCheck={false}
              placeholder="9.9.9.10"
              onChange={(e) => setBootstrap(e.target.value)}
            />
          </label>
          <label className="field">
            <span>Default upstream group</span>
            <select value={group} onChange={(e) => setGroup(e.target.value)}>
              {settings.groups.map((g) => (
                <option key={g} value={g}>
                  {g}
                </option>
              ))}
            </select>
          </label>
        </div>
        <SaveRow busy={busy} message={message} />
      </form>
    </Panel>
  );
}

/** How many days of query history and of per-minute statistics are kept. */
export function RetentionPanel({ settings }: { settings: Settings }) {
  const { save, message, setMessage, busy } = useSectionSave(settings);
  const [queryDays, setQueryDays] = useState(String(settings.retention.query_days));
  const [statsDays, setStatsDays] = useState(String(settings.retention.stats_days));

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const query = parseDays(queryDays);
    const stats = parseDays(statsDays);
    if (query === null || stats === null) {
      return setMessage({ tone: 'error', text: 'Enter a whole number of days, at least 1, in both boxes.' });
    }
    save({ retention: { query_days: query, stats_days: stats } }, 'Saved retention.');
  };

  return (
    <Panel
      title="History retention"
      subtitle="Older rows are deleted hourly. A shorter window deletes history, so lower it with care."
    >
      <form className="settings-form" onSubmit={onSubmit} noValidate>
        <div className="form-grid">
          <label className="field">
            <span>Keep the query log for (days)</span>
            <input inputMode="numeric" value={queryDays} onChange={(e) => setQueryDays(e.target.value)} />
          </label>
          <label className="field">
            <span>Keep statistics for (days)</span>
            <input inputMode="numeric" value={statsDays} onChange={(e) => setStatsDays(e.target.value)} />
          </label>
        </div>
        <SaveRow busy={busy} message={message} />
      </form>
    </Panel>
  );
}

/** Forces safe search for every client that does not set its own. */
export function SafeSearchPanel({ settings }: { settings: Settings }) {
  const { save, message, busy } = useSectionSave(settings);
  const [on, setOn] = useState(settings.safe_search);

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    save({ safe_search: on }, on ? 'Safe search is on.' : 'Safe search is off.');
  };

  return (
    <Panel
      title="Safe search"
      subtitle="Makes Google, Bing, DuckDuckGo and YouTube return filtered results. A client can override this on the Clients page."
    >
      <form className="settings-form" onSubmit={onSubmit} noValidate>
        <label className="checkbox">
          <input type="checkbox" checked={on} onChange={(e) => setOn(e.target.checked)} />
          <span>Force safe search for every client that does not set its own</span>
        </label>
        <span className="muted small">
          It does not stop a device that uses its own encrypted DNS, and it only covers those four providers.
        </span>
        <SaveRow busy={busy} message={message} />
      </form>
    </Panel>
  );
}

/** Downloads every setting as one YAML file, to keep a copy or move to another install. */
export function ExportPanel() {
  const [message, setMessage] = useState<Message | null>(null);
  const [busy, setBusy] = useState(false);

  const onExport = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const yaml = await api.exportSettings();
      const filename = exportFilename(new Date());
      const url = URL.createObjectURL(new Blob([yaml], { type: 'text/yaml' }));
      const link = document.createElement('a');
      link.href = url;
      link.download = filename;
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
      setMessage({ tone: 'ok', text: `Downloaded ${filename}.` });
    } catch (err) {
      setMessage({ tone: 'error', text: err instanceof Error ? err.message : 'Could not export the settings.' });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Panel
      title="Export settings"
      subtitle="Download resolvers, groups, clients, enabled lists, local answers, allow and block domains, bootstrap, the default group, retention and listeners as one YAML file."
    >
      <div className="settings-form">
        <p className="muted small">
          It does not include user accounts, passwords or query history. It does contain your resolver URLs (some include
          a personal profile ID), client networks and local answers, so check it before you share it.
        </p>
        {message && <Notice tone={message.tone}>{message.text}</Notice>}
        <div className="row-actions">
          <button type="button" className="btn btn-primary" disabled={busy} onClick={onExport}>
            {busy ? 'Preparing...' : 'Export settings'}
          </button>
        </div>
      </div>
    </Panel>
  );
}

/** Read only. Changing a port needs a restart and can lock you out of the dashboard. */
export function ListenersPanel({ settings }: { settings: Settings }) {
  return (
    <Panel
      title="Listeners"
      subtitle="Where AlwaysDNS listens. To change these, edit listen in config.yaml and restart the container."
    >
      <dl className="listen-list">
        <dt>DNS (UDP and TCP)</dt>
        <dd className="mono">{settings.listen.dns}</dd>
        <dt>Web UI and API</dt>
        <dd className="mono">{settings.listen.http}</dd>
      </dl>
    </Panel>
  );
}
