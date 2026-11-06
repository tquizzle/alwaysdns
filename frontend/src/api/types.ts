// Response and request types for the DNS backend API. These mirror the
// contract agreed with the backend. Fields marked "assumed" were not specified
// in the contract and are handled defensively in the UI.

export interface Me {
  username: string;
  role: string; // "admin" unlocks list editing
}

export interface LoginBody {
  username: string;
  password: string;
}

export type RangeKey = 'hour' | 'day' | 'week' | 'month' | 'year';

export interface TimelinePoint {
  /** Epoch seconds or milliseconds, or an ISO-8601 string (assumed). */
  t: string | number;
  total: number;
  blocked: number;
}

export interface NameCount {
  name: string;
  count: number;
  /** For blocked domains: the block source that matched most often (a list name, or a rule group). */
  reason?: string;
}

export interface UpstreamStatus {
  /** The resolver's friendly name, or the URL when it has none. */
  name: string;
  /** The server URL. Absent when the stats come from a source that does not report it. */
  url?: string;
  protocol?: Protocol;
  /** False after three failures in a row, live or probe. */
  ok: boolean;
  /** Average of live client queries, in ms. */
  avg_ms: number;
  errors: number;
  /** Median and 95th percentile of the recent probes, in ms. Null before the first probe. */
  probe_p50_ms?: number | null;
  probe_p95_ms?: number | null;
  probe_samples?: number;
}

export interface Stats {
  total: number;
  blocked: number;
  cached: number;
  forwarded: number;
  /** Assumed: fraction in the range 0 to 1. */
  block_rate: number;
  avg_ms: number;
  active_clients: number;
  timeline: TimelinePoint[];
  top_domains: NameCount[];
  top_blocked: NameCount[];
  upstreams: UpstreamStatus[];
  /** Assumed absent on older servers, so the UI treats a missing list as empty. */
  root_domains?: NameCount[];
  /** Query counts per company: Google, Apple, Microsoft, Amazon, Meta. */
  companies?: NameCount[];
  /** Blocked queries per block source. */
  block_reasons?: NameCount[];
  /** Query counts per destination country, named by ISO code. */
  countries?: NameCount[];
  /** False when the server has no country database, so the map has no data. */
  geo_ready?: boolean;
}

export interface QueryLogEntry {
  /** Unix milliseconds. */
  time: number;
  client: string;
  domain: string;
  qtype: string;
  /** Known values: "blocked" | "cached" | "forwarded" | "allowed" | "rewritten" | "error". Others render neutrally. */
  result: string;
  upstream: string;
  /** The resolver's friendly name, when a resolver has this upstream's URL. */
  upstream_name?: string;
  elapsed_ms: number;
  /** Matched rule, or empty string when none. */
  rule: string;
}

/** Result filter values. The empty string means all results. */
export type ResultFilter = '' | 'blocked' | 'cached' | 'forwarded' | 'allowed' | 'rewritten' | 'error';

export interface QueryLogParams {
  limit?: number;
  offset?: number;
  domain?: string;
  client?: string;
  result?: string;
  /** Unix milliseconds. */
  from?: number;
  /** Unix milliseconds. */
  to?: number;
}

export interface QueryLogResponse {
  items: QueryLogEntry[];
  /** Total matching entries, across all pages. */
  total: number;
}

export type ListKind = 'block' | 'allow';

export interface FilterList {
  id: number;
  kind: ListKind;
  name: string;
  url: string;
  enabled: boolean;
  rules: number;
  last_updated: string | null;
  error: string | null;
  /** Block lists, admin only: the client names that block with this list. */
  assigned_clients?: string[];
  /** Block lists, admin only: whether devices that match no client block with this list. */
  assigned_default?: boolean;
}

export interface ListsResponse {
  block: FilterList[];
  allow: FilterList[];
}

export interface AssignBody {
  /** The block list name, shared by its sources. */
  name: string;
  clients: string[];
  default: boolean;
}

export interface AssignResponse {
  ok: boolean;
  name: string;
  clients: string[];
  default: boolean;
}

export interface BlockBody {
  domain: string;
  /** A configured client name. Omit to block for every device. */
  client?: string;
}

export interface BlockResponse {
  ok: boolean;
  domain: string;
  scope: string;
}

export interface ListCreateBody {
  kind: ListKind;
  name: string;
  url: string;
  enabled: boolean;
}

export type ListUpdateBody = Partial<Omit<ListCreateBody, 'kind'>>;

export interface OkResponse {
  ok: boolean;
}

export interface RefreshListsResponse {
  ok: boolean;
  /** Number of lists that were refreshed. */
  lists: number;
}

export type Bucket = 'minute' | 'hour' | 'day';

export interface HistoryParams {
  /** Unix milliseconds (assumed; matches the query log). */
  from: number;
  /** Unix milliseconds (assumed; matches the query log). */
  to: number;
  bucket: Bucket;
}

export interface HistoryPoint {
  /** Unix seconds. */
  t: number;
  total: number;
  blocked: number;
}

export interface HistoryResponse {
  timeline: HistoryPoint[];
}

export interface Client {
  name: string;
  /** Admin only. Array of CIDR strings, e.g. "192.168.1.0/24". */
  cidrs?: string[];
  /** Admin only: this client's own allow entries, applied before block lists. */
  allow_domains?: string[];
  /** Admin only: this client's own block entries. */
  block_domains?: string[];
  upstream_group: string;
  /** Block lists this client uses. */
  block_lists: string[];
  /** Apps blocked for this client, by catalogue id. */
  blocked_services: string[];
  queries: number;
  blocked: number;
}

/** DNS-over-HTTPS or DNS-over-QUIC, from the URL scheme. */
export type Protocol = 'doh' | 'doq';

/** Admin only: a named upstream server and the groups that list it. */
export interface Resolver {
  name: string;
  url: string;
  protocol: Protocol | null;
  groups: string[];
}

/** Admin only: an upstream group and the clients that name it. */
export interface UpstreamGroup {
  name: string;
  /** Resolver names, in failover order. */
  resolvers: string[];
  clients: string[];
  is_default: boolean;
}

export interface ClientsResponse {
  clients: Client[];
  upstream_groups: UpstreamGroup[];
  resolvers: Resolver[];
}

/** The full rule set for one client. Every save sends all of it. `name` is used only when creating. */
export interface ClientUpdateBody {
  name?: string;
  cidrs: string[];
  upstream_group: string;
  block_lists: string[];
  allow_domains: string[];
  block_domains: string[];
  blocked_services: string[];
}

/** A local answer: a name, or `*.suffix`, that is answered with this address instead of asking an upstream. */
export interface Rewrite {
  domain: string;
  answer: string;
}

export interface Retention {
  query_days: number;
  stats_days: number;
}

/** Admin only: what the Settings page shows. `listen` and `groups` are read-only context. */
export interface Settings {
  rewrites: Rewrite[];
  allow_domains: string[];
  block_domains: string[];
  bootstrap: string[];
  default_upstream_group: string;
  retention: Retention;
  listen: { dns: string; http: string };
  groups: string[];
}

/** What a save sends: every editable setting, so a missing field cannot clear one. */
export type SettingsBody = Omit<Settings, 'listen' | 'groups'>;

/** An app a client can block, from the catalogue. `domains` is how many domains it covers. */
export interface ServiceItem {
  id: string;
  name: string;
  group: string;
  domains: number;
}

export interface ServicesResponse {
  services: ServiceItem[];
  /** Set when the cached catalogue is in use because the download failed. */
  note: string | null;
}

export interface GroupCreateBody {
  name: string;
  /** Resolver names, in failover order. */
  resolvers: string[];
}

export interface GroupUpdateBody {
  resolvers: string[];
}

/** Adds a resolver, or changes one. A different `name` on update renames it. */
export interface ResolverBody {
  name: string;
  url: string;
}

export interface Health {
  ok: boolean;
  version: string;
}
