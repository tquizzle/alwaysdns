# DNS Control frontend

Web UI for the homelab DNS server: dashboard, query log, clients and login.

Stack: Vite, React 19, TypeScript, TanStack Query (polling), TanStack Table, hand-drawn SVG charts.

## Commands

```bash
npm install        # install dependencies (.npmrc sets legacy-peer-deps)
npm run dev        # dev server on http://localhost:5173, /api proxied to http://localhost:8080
npm run build      # type-check, then build to dist/
npm run preview    # serve dist/ locally (also proxies /api)
npm test           # run Vitest unit tests
```

To point the dev proxy at another backend:

```bash
DNSUI_API_TARGET=http://192.168.1.10:8080 npm run dev
```

## Layout

- `src/api/`: typed API client, endpoint functions, TanStack Query hooks.
- `src/pages/`: Dashboard, QueryLog, Lists, Clients, Login.
- `src/components/`: gauges, line chart, bar lists, tables, states, error boundary.
- `src/lib/`: formatting helpers, display-only redaction, hash router, preferences.

Polling: stats every 5 s, query log every 3 s while Live is on, clients every 30 s, health every 15 s.
A 401 from any protected endpoint returns the user to the login page.

## Pages

- **Dashboard**: KPIs and charts for the hour, day, week, month or year range (`/api/stats?range=`). The History panel
  picks a from and to date and a bucket (minute, hour or day) and draws the timeline from `/api/history`.
- **Query log**: server-side paging (50 or 100 per page) with filters for domain, client, result and time range
  (last hour, day, week or month, or a custom from and to). Live refresh applies to the first page only.
- **Lists**: block and allow lists with enable toggles, rule counts, last updated times and errors. Admins can add,
  edit, delete and refresh lists. Viewers see read-only lists with the URL masked (credentials and query values hidden).
- **Clients** is unchanged.
