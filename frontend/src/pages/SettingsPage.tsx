import { useSettings } from '../api/queries';
import {
  DomainsPanel,
  ExportPanel,
  ListenersPanel,
  LocalAnswersPanel,
  ResolvingPanel,
  RetentionPanel,
} from '../components/SettingsPanels';
import { ErrorState, LoadingState } from '../components/States';

/** Admin page for the settings that have no page of their own. */
export function SettingsPage() {
  const settings = useSettings();

  return (
    <div className="page">
      <div className="page-bar">
        <div>
          <h1>Settings</h1>
          <p className="muted small">
            Local answers, global allow and block domains, and resolver defaults. Resolvers and groups are on the{' '}
            <a href="#/upstreams">Upstreams</a> page, and lists are on the <a href="#/lists">Lists</a> page.
          </p>
        </div>
      </div>

      {!settings.data && settings.isPending && <LoadingState label="Loading settings" />}
      {!settings.data && settings.isError && <ErrorState error={settings.error} onRetry={() => settings.refetch()} />}
      {settings.data && (
        <>
          <LocalAnswersPanel settings={settings.data} />
          <div className="settings-pair">
            <DomainsPanel settings={settings.data} field="allow_domains" />
            <DomainsPanel settings={settings.data} field="block_domains" />
          </div>
          <ResolvingPanel settings={settings.data} />
          <RetentionPanel settings={settings.data} />
          <ListenersPanel settings={settings.data} />
          <ExportPanel />
        </>
      )}
    </div>
  );
}
