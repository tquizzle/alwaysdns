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
- **Explicit memory bounds.** The answer cache holds at most 10,000 entries, the in-memory query log 5,000 rows, the write queue 10,000 rows, and live login sessions 256 (`backend/src/cache.rs`, `telemetry.rs`, `recorder.rs`, `auth.rs`). Retention deletes history hourly. Measured: the production container (`docker stats`, 2026-11-06) used 38.98 MiB of memory, 0.37% CPU and 8 processes while serving the LAN. Caveat: this is one snapshot. Uptime, query rate and block-list size were not recorded, so it says nothing about long-term growth. Block lists load fully into `HashSet<String>` and grow with list size, and the SQLite file stops growing once it holds one retention window, but never shrinks on its own (freed pages are reused, not returned). `dnsui vacuum` gives the space back on demand. Track memory over days before claiming it stays flat. Pi-hole documents 512 MB RAM as its minimum ([Pi-hole](https://docs.pi-hole.net/main/prerequisites/)), but that is a hardware requirement, not its measured use, so it is not a like-for-like comparison. No measured figure was found for the other resolvers.
- **One binary on musl.** Rust targets `x86_64-unknown-linux-musl` link the C runtime statically by default ([Rust Reference](https://doc.rust-lang.org/reference/linkage.html)). The runtime image is `alpine:3.20`, not scratch. Measured (`docker images`, 2026-11-06): `dnsui:latest` is 65.1 MB on disk (18.4 MB compressed content size), and that includes the web UI and Alpine.
- **Async DNS stack.** Tokio runs the listeners and background jobs. [hickory-dns](https://github.com/hickory-dns/hickory-dns) provides the DNS, DoH and DoQ code (`hickory-server`, `hickory-resolver` 0.26.3). Caveat: its README does not list production users. The project pulls in about 260 third-party crates (`backend/Cargo.lock`), which is a supply-chain cost.

What Rust does not buy: faster answers when the upstream resolver is the bottleneck. Upstream round-trip time usually dominates a cache miss.

## Compared with AdGuard Home and Pi-hole

AdGuard Home and Pi-hole are the mature choices. This section says where AlwaysDNS differs, and where it does not.

- **What AlwaysDNS has.** Named resolvers with their own status cards, error counts and p95 latency. Ordered failover groups that each client picks. DoH and DoQ upstreams. Per-client block lists and allow/block domains. A read-only viewer role that redacts client names and list URLs. An `import-adguard` command to move a config over. Caveat: this list comes from this PRD, not from a feature-by-feature check of the other two. AdGuard Home also has [per-client settings](https://github.com/AdguardTeam/AdGuardHome), so per-client control is not unique.
- **What it lacks.** AdGuard Home has a built-in DHCP server, parental control and forced safe search ([README](https://github.com/AdguardTeam/AdGuardHome)). AlwaysDNS has no DHCP server and no forced safe search yet (it is planned as a later phase). It covers part of parental control with per-client adult-content and app blocking, and does not use AdGuard's hosted parental control service. It also does not serve encrypted DNS to clients: its listeners are UDP and TCP port 53 only. It has no audit, no community and one maintainer.
- **Memory is a tie.** The production container measured 38.98 MiB (see Why Rust). A benchmark from a competing Rust DNS project reports about 39 MB idle for AdGuard Home ([source](https://awesome.ecosyste.ms/projects/github.com%2Femotionalamo%2Frust-dns-backend)). That is one interested source, so treat it as rough. Pi-hole's memory depends mostly on how many blocked domains it loads, plus per-domain metadata ([Pi-hole forum](https://discourse.pi-hole.net/t/what-determines-memory-usage/15652)). Users also report AdGuard Home memory spikes during filter updates ([GL-iNet forum](https://forum.gl-inet.com/t/adguard-home-memory-usage/65883)) and one report of 13.9 GiB after three days ([issue 6202](https://github.com/AdguardTeam/AdGuardHome/issues/6202)). These are single anecdotes. AlwaysDNS has not been measured over days, so it cannot claim better.
- **Language.** Pi-hole's FTL is C built on dnsmasq ([Pi-hole](https://pi-hole.net/?p=14497)). AdGuard Home is Go, which is also memory-safe. Rust's clear edge is memory safety against C, not against Go.
- **When to pick the others.** If you need DHCP, parental control, encrypted DNS serving or a large community, use AdGuard Home or Pi-hole. AlwaysDNS fits when you want its resolver cards, per-client failover groups and viewer role.

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
- **Block adult content** is a switch in the client dialog. It assigns the StevenBlack porn-only list (MIT licence, about 77,000 domains), which is added to the Lists page on first use.
- **Blocked apps** are chosen in the client dialog from a searchable, grouped list (YouTube, Discord, TikTok and so on). The catalogue has 142 apps, taken from AdGuard's HostlistsRegistry. It is downloaded at start-up and on the daily refresh, cached on disk, and never bundled, because its repository is GPL-3.0. A blocked app blocks its domains for that client only. The query log shows the rule as `list:app/<id>:<domain>`, and the blocked-by-list panel counts each app separately.
- Limits of app blocking: rules that are wildcards, start anchors or `$dnstype` are skipped (51 of 2,406 in the catalogue when checked), so an app block can miss some of its domains. DNS blocking is also not airtight, because apps with their own hardcoded DNS or IP addresses can bypass it. If the catalogue cannot be downloaded, the last loaded copy stays in use.

### Settings

An admin-only Settings page (`#/settings`) edits the config items that have no page of their own. Each section saves on its own and sends every other section unchanged, so one save cannot clear another.

- **Local answers.** Names, or `*.suffix` wildcards, answered with a fixed IPv4 or IPv6 address instead of asking an upstream. An exact name wins over a wildcard. Names are stored lowercase, and one name can have one answer.
- **Global allow domains.** Never blocked, for every device. They also cover subdomains and override every block list.
- **Global block domains.** Always blocked, for every device, including subdomains.
- **Resolver defaults.** The bootstrap IP addresses, and the default upstream group for devices that match no client. Bootstrap cannot be emptied while a resolver is given by hostname.
- **History retention.** Days of query log and of per-minute statistics. Both must be at least 1, because zero would delete all history. Lowering them deletes older rows at the next hourly pass.
- **Listeners** (DNS and web ports) are shown read-only. Changing a port needs a restart and can lock you out of the dashboard, so edit `listen` in `config.yaml` instead.
- **Export settings** downloads every setting as one YAML file named `alwaysdns-settings-<date>.yaml`. Lists live in the database, so the export takes the enabled lists from there, and a disabled list is left out. It never includes user accounts, password hashes, sessions or query history. It does include resolver URLs (some contain a personal profile ID), client networks and local answers, so check it before sharing. The file is a valid config, so it can be copied to `config.yaml` on another install.

Resolvers and groups are on the Upstreams page, clients on the Clients page, and lists on the Lists page, so every setting in the config file has an editor.

### Block and allow lists

- A list source is an http(s) URL or an absolute file path on the server.
  - URLs: the host is resolved before any request, and the source is refused if any address is loopback, unspecified, link-local or multicast (IPv4-mapped IPv6 included). Private LAN addresses are allowed. Redirects are capped at five hops and stop at a literal blocked IP.
  - Files: the path must resolve, after symlinks and `..`, inside the lists directory: `DNSUI_LISTS_DIR`, or `lists/` in the data directory. Other paths are refused. Existing rows outside it keep their place and show the error on refresh.
  - Known limit: a hostname that re-resolves to a blocked address after the check (DNS rebinding) is not caught, and a redirect to a hostname is not checked.
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
- Failed logins back off in memory, per username and per client IP. The first 5 failures are free, then each failure locks that key for 30 seconds, doubling up to 15 minutes; a locked login gets HTTP 429 with `Retry-After`. A good login clears the username's count. Tradeoff: anyone can lock a known username out for up to 15 minutes by guessing it.
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
- The app catalogue needs outbound HTTPS to `adguardteam.github.io`. Start-up waits for that download, which took about 10 seconds in a local test, and falls back to the cached copy if it fails.
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
6. Per-client parental controls: a Block adult content switch and a blocked-apps picker in the client dialog, backed by the AdGuard services catalogue. Forced safe search is the planned next phase and is not built.
7. Settings page: local answers, global allow and block domains, resolver defaults and retention are editable in the UI. Only the listener ports stay in `config.yaml`.

### Still open

- Deploy the new build to the server.
- Keyboard and touch access for chart tooltips.
- An unsaved-changes warning when a dialog is closed with Escape.
