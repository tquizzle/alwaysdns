import { useCallback, useSyncExternalStore } from 'react';

export const ROUTES = ['dashboard', 'querylog', 'lists', 'upstreams', 'clients', 'settings'] as const;
export type Route = (typeof ROUTES)[number];

function currentRoute(): Route {
  // A query such as #/querylog?domain=example.com carries a starting filter, not a different page.
  const raw = window.location.hash.replace(/^#\/?/, '').split('?')[0] ?? '';
  return (ROUTES as readonly string[]).includes(raw) ? (raw as Route) : 'dashboard';
}

/** A value from the hash's query part, such as `domain` in #/querylog?domain=example.com. */
export function hashParam(name: string): string | null {
  const query = window.location.hash.split('?')[1];
  return query ? new URLSearchParams(query).get(name) : null;
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener('hashchange', onChange);
  return () => window.removeEventListener('hashchange', onChange);
}

/** Minimal hash router: #/dashboard, #/querylog, #/lists, #/upstreams, #/clients. */
export function useHashRoute(): [Route, (route: Route) => void] {
  const route = useSyncExternalStore(subscribe, currentRoute, () => 'dashboard' as Route);
  const navigate = useCallback((next: Route) => {
    window.location.hash = `/${next}`;
  }, []);
  return [route, navigate];
}
