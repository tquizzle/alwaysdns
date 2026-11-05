import { useCallback, useSyncExternalStore } from 'react';

export const ROUTES = ['dashboard', 'querylog', 'lists', 'upstreams', 'clients'] as const;
export type Route = (typeof ROUTES)[number];

function currentRoute(): Route {
  const raw = window.location.hash.replace(/^#\/?/, '');
  return (ROUTES as readonly string[]).includes(raw) ? (raw as Route) : 'dashboard';
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
