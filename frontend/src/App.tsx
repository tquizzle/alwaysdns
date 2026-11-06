import { useEffect } from 'react';
import { useMe, useHealth } from './api/queries';
import { ErrorBoundary } from './components/ErrorBoundary';
import { ErrorState, LoadingState } from './components/States';
import { UserMenu } from './components/UserMenu';
import { useHashRoute, type Route } from './lib/hashRoute';
import { BrandMark, LoginPage } from './pages/LoginPage';
import { Dashboard } from './pages/Dashboard';
import { QueryLogPage } from './pages/QueryLogPage';
import { ClientsPage } from './pages/ClientsPage';
import { ListsPage } from './pages/ListsPage';
import { UpstreamsPage } from './pages/UpstreamsPage';

const NAV: { route: Route; label: string; adminOnly?: boolean }[] = [
  { route: 'dashboard', label: 'Dashboard' },
  { route: 'querylog', label: 'Query log' },
  { route: 'lists', label: 'Lists' },
  { route: 'upstreams', label: 'Upstreams', adminOnly: true },
  { route: 'clients', label: 'Clients' },
];

export function App() {
  const me = useMe();

  if (me.isPending) {
    return (
      <div className="boot">
        <LoadingState label="Checking session" />
      </div>
    );
  }

  if (me.isError) {
    return (
      <div className="boot">
        <ErrorState error={me.error} onRetry={() => me.refetch()} />
      </div>
    );
  }

  if (!me.data) {
    return <LoginPage />;
  }

  return <Shell username={me.data.username} role={me.data.role} />;
}

function Shell({ username, role }: { username: string; role: string }) {
  const [requested, navigate] = useHashRoute();
  const isAdmin = role === 'admin';
  // Viewers have no Upstreams page. A pasted #/upstreams link falls back to the dashboard.
  const route: Route = requested === 'upstreams' && !isAdmin ? 'dashboard' : requested;

  // The tab title names the page, so several open tabs can be told apart.
  useEffect(() => {
    const label = NAV.find((item) => item.route === route)?.label ?? 'Dashboard';
    document.title = `${label} · AlwaysDNS`;
  }, [route]);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <BrandMark />
          <span>AlwaysDNS</span>
        </div>

        <nav className="nav" aria-label="Main">
          {NAV.filter((item) => !item.adminOnly || isAdmin).map((item) => (
            <a
              key={item.route}
              href={`#/${item.route}`}
              aria-current={route === item.route ? 'page' : undefined}
              className={route === item.route ? 'active' : ''}
              onClick={(e) => {
                e.preventDefault();
                navigate(item.route);
              }}
            >
              {item.label}
            </a>
          ))}
        </nav>

        <div className="topbar-right">
          <HealthPill />
          <UserMenu username={username} role={role} />
        </div>
      </header>

      <main className="content">
        <ErrorBoundary resetKey={route}>
          {route === 'dashboard' && <Dashboard />}
          {route === 'querylog' && <QueryLogPage />}
          {route === 'lists' && <ListsPage isAdmin={isAdmin} />}
          {route === 'upstreams' && <UpstreamsPage />}
          {route === 'clients' && <ClientsPage />}
        </ErrorBoundary>
      </main>
    </div>
  );
}

function HealthPill() {
  const health = useHealth();
  if (health.isError) {
    return <span className="pill pill-red" role="status">API offline</span>;
  }
  if (!health.data) {
    return <span className="pill" role="status">Checking</span>;
  }
  return (
    <span className={`pill ${health.data.ok ? 'pill-green' : 'pill-yellow'}`} role="status">
      {health.data.ok ? 'Healthy' : 'Degraded'} · v{health.data.version}
    </span>
  );
}
