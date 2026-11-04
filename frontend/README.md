# DNS Control frontend

Web UI for the homelab DNS server: dashboard, query log, lists, clients and login.

Stack: Vite, React 19, TypeScript, TanStack Query (polling), TanStack Table, hand-drawn SVG charts.

## Commands

```bash
npm install        # install dependencies
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
- `src/components/`: gauges, KPI cards, line chart, bar lists, company donut, traffic map, tables, dialogs, states, error boundary.
- `src/lib/`: formatting helpers, display-only redaction, hash router, colour palettes, preferences.

Routes are hash based: `#/dashboard`, `#/querylog`, `#/lists`, `#/clients`. Unknown hashes open the dashboard.

Polling: stats every 5 s, query log every 3 s while Live is on, clients every 30 s, health every 15 s.
A 401 from any protected endpoint returns the user to the login page.

## Pages

- **Dashboard**: for the hour, day, week, month or year range (`/api/stats?range=`): six KPI cards with full counts,
  a block-rate gauge, a queries and blocks chart (the interval still filling is drawn dashed), top domains, root domains,
  top blocked domains, block reasons, a company share donut, a traffic destination map, upstream health and a live query
  log. The History panel picks a from and to date and a bucket (per minute, hour or day) and draws the timeline from
  `/api/history`.
- **Query log**: server-side paging (50 or 100 per page) with filters for domain, client, result and time range
  (last hour, day, week or month, or a custom from and to). Live refresh applies to the first page only. Admins get a
  Block column with "This device" and "Everyone" buttons.
- **Lists**: block and allow lists with enable toggles, rule counts, last updated times, errors and per-client
  assignment chips. Admins can add, edit, delete and refresh lists and change assignments. Viewers see read-only lists
  with the URL masked (credentials and query values hidden). Admins get the same masking while Mask IPs is on.
- **Clients**: configured clients with their networks, upstream group, block-list assignments and query counts, plus an
  upstream groups panel. Admins can add, edit and delete clients and groups. A group that is in use cannot be deleted.
- **Top bar**: health status and version, a colour palette picker, and a Mask IPs toggle that hides the last two octets
  of client addresses on screen. Both are saved in the browser.
