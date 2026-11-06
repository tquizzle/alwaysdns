Title: AlwaysDNS
Author: tquizzle
Technology Stack: Rust backend (SQLite); React 19, Vite, TypeScript, TanStack Query and Table frontend; Docker Compose

It's always DNS. This is the homelab resolver that lets you see where.

## Overview

AlwaysDNS is a self-hosted, filtering DNS resolver with a web dashboard, built for a homelab. It answers DNS on UDP and TCP port 53, forwards queries to named upstream resolvers over DNS-over-HTTPS or DNS-over-QUIC, blocks domains using block lists, and shows what each device is asking for. The web UI and API listen on port 8080.

## Why Rust

The resolver sits on the path of every DNS query on the LAN, so the backend language matters. These reasons are trade-offs, not free wins.

- **Memory safety in safe code.** The compiler rejects use-after-free and buffer overflows in safe Rust. Microsoft reported that about 70% of its patched CVEs over 12 years were memory-safety bugs (2019 BlueHat talk, [ZDNet](https://www.zdnet.com/article/microsoft-70-percent-of-all-security-bugs-are-memory-safety-issues)). Google reports memory-safety bugs below 20% of Android's vulnerabilities in 2025 as Rust code grew ([Google](https://blog.google/security/rust-in-android-move-fast-fix-things/)). Caveat: those are large C/C++ codebases. This backend has no `unsafe` blocks, but the binary still links C: bundled SQLite (`rusqlite` `bundled`, compiled from C source by `cc`, per the [rusqlite README](https://github.com/rusqlite/rusqlite)) and the TLS/crypto stack.
- **No garbage-collector pauses.** Discord measured latency and CPU spikes about every two minutes in a Go service, caused by forced GC runs and scanning a large LRU cache ([Discord](https://discord.com/blog/why-discord-is-switching-from-go-to-rust)). Caveat: that is one workload from 2020. This project has not measured its own tail latency.
- **Explicit memory bounds.** The answer cache holds at most 10,000 entries, the in-memory query log 5,000 rows, the write queue 10,000 rows, and live login sessions 256 (`backend/src/cache.rs`, `telemetry.rs`, `recorder.rs`, `auth.rs`). Retention deletes history hourly. Measured: the production container (`docker stats`, 2026-11-06) used 38.98 MiB of memory, 0.37% CPU and 8 processes while serving the LAN. Caveat: this is one snapshot. Uptime, query rate and block-list size were not recorded, so it says nothing about long-term growth. Block lists load fully into `HashSet<String>` and grow with list size, and the SQLite file is never vacuumed. Track memory over days before claiming it stays flat. Pi-hole documents 512 MB RAM as its minimum ([Pi-hole](https://docs.pi-hole.net/main/prerequisites/)), but that is a hardware requirement, not its measured use, so it is not a like-for-like comparison. No measured figure was found for the other resolvers.
- **One binary on musl.** Rust targets `x86_64-unknown-linux-musl` link the C runtime statically by default ([Rust Reference](https://doc.rust-lang.org/reference/linkage.html)). The runtime image is `alpine:3.20`, not scratch. Measured (`docker images`, 2026-11-06): `dnsui:latest` is 65.1 MB on disk (18.4 MB compressed content size), and that includes the web UI and Alpine.
- **Async DNS stack.** Tokio runs the listeners and background jobs. [hickory-dns](https://github.com/hickory-dns/hickory-dns) provides the DNS, DoH and DoQ code (`hickory-server`, `hickory-resolver` 0.26.3). Caveat: its README does not list production users. The project pulls in about 260 third-party crates (`backend/Cargo.lock`), which is a supply-chain cost.

What Rust does not buy: faster answers when the upstream resolver is the bottleneck. Upstream round-trip time usually dominates a cache miss.

## Naming

- The product is AlwaysDNS. The code and deploy files still use the name dnsui (container, env vars, volume, package names and paths).
- The in-app UI text (browser tab titles, the top bar and the login page) says AlwaysDNS.

## Features

### Resolvers and upstream groups

- A resolver is a friendly name plus one URL. The protocol comes from the scheme: `https://` is DoH, `quic://` is DoQ.
- Each resolver has its own stats and its own status card on the dashboard. Renaming keeps its stats. Changing its URL resets them.
- An upstream group is an ordered list of resolver names. The order is the failover order.
- Clients pick a group. Clients without one use the default group.
- A resolver cannot be deleted while a group lists it. A group cannot be deleted while clients use it, and the default group cannot be deleted.
- Admins manage both on the Upstreams page (`#/upstreams`): resolvers first, then upstream groups. The Clients page links there and does not hold these panels.

### Clients

- A client is matched by one or more CIDR networks.
- Each client has an upstream group, assigned block lists, and its own allow and block domains.
- Devices that match no client use the default block lists.
- Admins add, edit and delete clients on the Clients page.

### Block and allow lists

- A list source is an http(s) URL or an absolute file path on the server.
- Formats: auto, hosts, plain, abp.
- Lists can be added, edited, deleted, enabled or disabled, and refreshed. Each list shows its rule count, last updated time and any error.
- Block lists are assigned per client, plus a default assignment for unknown devices.
- Allow lists override block lists for every client.

### Query log

- Server-side paging (50 or 100 rows per page).
- Filters: domain (admin only), client, result, and time range (preset or custom).
- Live refresh applies to the first page only. It pauses while the pointer or focus is on the table, so rows do not move under the cursor.
- The upstream column shows the resolver's friendly name, with the URL in the tooltip.
- Admins get one "Block…" button per row. It opens a choice of "This device" or "Everyone".

### Dashboard

- Range: hour, day, week, month or year.
- Top row: a green-to-blue Total queries orb on the left, six KPI cards in the middle, and a red Block rate orb on the right.
- Resolvers panel: one full-size card per resolver, with status and latency rings, errors and p95 latency.
- Queries and blocks timeline.
- Top domains and root domains. Domain names link to the query log filtered by that domain (`#/querylog?domain=...`).
- Blocked panel: top blocked domains (with the list that blocked each, linked to the query log) and blocked queries by list.
- Live query log.
- Traffic insight, collapsed by default: a GAFAM share donut and a traffic destination map. The map uses a local GeoLite2-Country database. Without it the panel shows a hint.
- History panel (admin): pick a date range and a bucket of minute, hour or day.
- Refresh intervals: stats every 5 s, query log every 3 s while Live is on, clients every 30 s, health every 15 s.
- A banner appears when a refresh fails, showing the time of the last good update.

### Access and display

- Login with a session cookie. Sessions last 12 hours.
- Two roles:
  - Admin: full access, including configuration changes, block actions and the Upstreams page.
  - Viewer: read-only. Client names are redacted, list URLs are masked, list errors show only "failed", and the domain filter and the Upstreams page are unavailable.
- Top bar: health status and version, and an account menu. The menu holds the colour palette picker, a Mask IPs toggle and Sign out.
- Mask IPs (saved in the browser) hides the last two octets of client addresses. Admins also see masked list URLs while it is on.
- Modal dialogs trap focus while open and return it to the element that opened them on close.
- On phones, tables stack into cards.

## Configuration

- The config file is `config.yaml` in the deploy directory, mounted into the container.
- After the first start, the database is the source of truth. The YAML `lists` and `allow_lists` sections are imported once, into an empty lists table. Later edits to those sections have no effect.
- Changes made in the dashboard are saved back to `config.yaml`.
- The dashboard has no raw config editor. Apply a manual edit by restarting the container or calling `POST /api/reload`.
- A config from an older version that lists raw URLs in groups is converted on first start. Each URL becomes a resolver named after its hostname, and the old file is kept as `config.yaml.bak`. Rename the resolvers on the Upstreams page.
- Other sections: listeners, bootstrap resolvers, clients, local rewrites, global allow and block domains, default block lists, default upstream group, and retention (query log 30 days, stats 365 days by default).

## Deployment

- Docker Compose in `deploy/`: one container for DNS, the web UI and the API, with a healthcheck and a persistent data volume.
- A Caddy site on another host serves the UI at `https://dns.example.test`.
- Port 53 must be free on the host. On Ubuntu this usually means disabling systemd-resolved.
- `quic://` upstreams need outbound UDP 853. If it is blocked, those queries time out. Change the resolver to its `https://` (DoH) URL and reload.
- The destination map needs a MaxMind GeoLite2-Country database. It is not included, because MaxMind does not allow redistribution. Copy it into the data volume and restart.
- Updates: `docker compose up -d --build`. The data volume keeps its contents across rebuilds.

## Status (not yet deployed)

### Done on main, not yet deployed to the server

The UI/UX overhaul from the design review is complete in the code on the main branch. None of it is on the server yet.

1. Accessibility and safety: modal focus handling, a two-step Block action in the query log, live refresh that pauses while the table is in use, larger touch targets and text.
2. Responsive layouts: tables stack into cards on phones, and account controls sit in a menu.
3. Resolvers, groups and clients flow: an Upstreams page for resolvers, then groups, with the Clients page linking to it.
4. Dashboard redesign: the total queries and block rate orbs, full-size resolver cards, the merged Blocked panel, and a collapsed Traffic insight section.
5. Styling polish and shared design tokens: colour tokens and a trimmed type scale.

### Still open

- Deploy the new build to the server.
- Keyboard and touch access for chart tooltips.
- An unsaved-changes warning when a dialog is closed with Escape.
