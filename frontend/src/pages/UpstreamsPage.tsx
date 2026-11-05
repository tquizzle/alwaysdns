import { useState } from 'react';
import { useClients } from '../api/queries';
import { ResolversPanel } from '../components/ResolversPanel';
import { ErrorState, LoadingState } from '../components/States';
import { UpstreamGroupsPanel } from '../components/UpstreamGroupsPanel';

/** Admin page: resolvers first, then the groups that order them. Clients pick a group on the Clients page. */
export function UpstreamsPage() {
  const clients = useClients();
  // Raised by the group form when it needs a resolver that does not exist yet.
  const [newResolverSignal, setNewResolverSignal] = useState(0);

  return (
    <div className="page">
      <div className="page-bar">
        <div>
          <h1>Upstreams</h1>
          <p className="muted small">
            Add resolvers, put them in order as groups, then give each client a group on the{' '}
            <a href="#/clients">Clients</a> page.
          </p>
        </div>
      </div>

      {!clients.data && clients.isPending && <LoadingState label="Loading upstreams" />}
      {!clients.data && clients.isError && <ErrorState error={clients.error} onRetry={() => clients.refetch()} />}
      {clients.data && (
        <>
          <ResolversPanel resolvers={clients.data.resolvers} openSignal={newResolverSignal} />
          <UpstreamGroupsPanel
            groups={clients.data.upstream_groups}
            resolvers={clients.data.resolvers}
            onNewResolver={() => setNewResolverSignal((n) => n + 1)}
          />
        </>
      )}
    </div>
  );
}
