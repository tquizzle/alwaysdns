import { useEffect, useState } from 'react';
import { useConfig, useReloadConfig, useSaveConfig } from '../api/queries';
import { Panel } from '../components/Panel';
import { ErrorState, LoadingState } from '../components/States';
import { redactYaml } from '../lib/redact';

interface Props {
  isAdmin: boolean;
}

export function ConfigPage({ isAdmin }: Props) {
  if (!isAdmin) {
    return (
      <div className="page">
        <Panel title="Admin only">
          <p className="muted">Your account does not have access to the server configuration.</p>
        </Panel>
      </div>
    );
  }
  return <ConfigEditor />;
}

function ConfigEditor() {
  const config = useConfig(true);
  const save = useSaveConfig();
  const reload = useReloadConfig();

  const [draft, setDraft] = useState('');
  const [touched, setTouched] = useState(false);
  const [maskPreview, setMaskPreview] = useState(true);
  const [notice, setNotice] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null);

  // Seed the editor from the server, but never overwrite what the user is typing.
  useEffect(() => {
    if (config.data && !touched) setDraft(config.data.yaml);
  }, [config.data, touched]);

  const dirty = config.data !== undefined && draft !== config.data.yaml;

  const onSave = () => {
    setNotice(null);
    save.mutate(draft, {
      onSuccess: () => {
        setTouched(false);
        setNotice({ tone: 'ok', text: 'Configuration saved. Use Reload to apply it.' });
      },
      onError: (err) => setNotice({ tone: 'error', text: err.message }),
    });
  };

  const onReload = () => {
    setNotice(null);
    reload.mutate(undefined, {
      onSuccess: () => setNotice({ tone: 'ok', text: 'Configuration reloaded.' }),
      onError: (err) => setNotice({ tone: 'error', text: err.message }),
    });
  };

  return (
    <div className="page">
      <div className="page-bar">
        <div>
          <h1>Configuration</h1>
          <p className="muted small">Edit the YAML, save it, then reload to apply. Saved changes are not live until reloaded.</p>
        </div>
        <div className="btn-row">
          <button type="button" className="btn" onClick={onReload} disabled={reload.isPending || dirty}>
            {reload.isPending ? 'Reloading...' : 'Reload'}
          </button>
          <button type="button" className="btn btn-primary" onClick={onSave} disabled={!dirty || save.isPending}>
            {save.isPending ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>

      {notice && (
        <div className={`state ${notice.tone === 'ok' ? 'state-ok' : 'state-error'}`} role="status">
          {notice.text}
        </div>
      )}
      {dirty && <p className="small text-yellow">You have unsaved changes.</p>}
      {reload.isError && !notice && <ErrorState error={reload.error} />}

      {config.isPending && !config.data && <LoadingState label="Loading configuration" />}
      {config.isError && !config.data && <ErrorState error={config.error} onRetry={() => config.refetch()} />}

      {config.data && (
        <div className="config-grid">
          <Panel title="YAML">
            <textarea
              className="yaml-editor"
              aria-label="Server configuration YAML"
              value={draft}
              spellCheck={false}
              onChange={(e) => {
                setTouched(true);
                setDraft(e.target.value);
              }}
              rows={24}
            />
          </Panel>

          <Panel
            title="Preview"
            actions={
              <button
                type="button"
                className="btn btn-small"
                aria-pressed={maskPreview}
                onClick={() => setMaskPreview((v) => !v)}
              >
                {maskPreview ? 'Secrets masked' : 'Showing secrets'}
              </button>
            }
          >
            <pre className="yaml-preview">{maskPreview ? redactYaml(draft) : draft}</pre>
          </Panel>
        </div>
      )}
    </div>
  );
}
