import { apiRequest } from './client';
import { buildQueryLogQuery } from '../lib/querylog';
import type {
  AssignBody,
  AssignResponse,
  BlockBody,
  BlockResponse,
  ClientUpdateBody,
  ClientsResponse,
  FilterList,
  GroupCreateBody,
  GroupUpdateBody,
  Health,
  HistoryParams,
  HistoryResponse,
  ListCreateBody,
  ListUpdateBody,
  ListsResponse,
  LoginBody,
  Me,
  OkResponse,
  QueryLogParams,
  QueryLogResponse,
  RangeKey,
  RefreshListsResponse,
  ResolverBody,
  Stats,
} from './types';

export const api = {
  login: (body: LoginBody) =>
    apiRequest<Me>('/api/auth/login', { method: 'POST', json: body, skipUnauthorized: true }),

  me: () => apiRequest<Me>('/api/auth/me', { skipUnauthorized: true }),

  logout: () => apiRequest<void>('/api/auth/logout', { method: 'POST', skipUnauthorized: true }),

  stats: (range: RangeKey) =>
    apiRequest<Stats>(`/api/stats?range=${encodeURIComponent(range)}`),

  queryLog: (params: QueryLogParams = {}) => {
    const query = buildQueryLogQuery(params);
    return apiRequest<QueryLogResponse>(`/api/querylog${query ? `?${query}` : ''}`);
  },

  history: (params: HistoryParams) => {
    const qs = new URLSearchParams({
      from: String(params.from),
      to: String(params.to),
      bucket: params.bucket,
    });
    return apiRequest<HistoryResponse>(`/api/history?${qs.toString()}`);
  },

  lists: () => apiRequest<ListsResponse>('/api/lists'),

  createList: (body: ListCreateBody) =>
    apiRequest<FilterList>('/api/lists', { method: 'POST', json: body }),

  updateList: (id: number, body: ListUpdateBody) =>
    apiRequest<FilterList>(`/api/lists/${id}`, { method: 'PUT', json: body }),

  deleteList: (id: number) => apiRequest<OkResponse>(`/api/lists/${id}`, { method: 'DELETE' }),

  refreshLists: () => apiRequest<RefreshListsResponse>('/api/lists/refresh', { method: 'POST' }),

  clients: () => apiRequest<ClientsResponse>('/api/clients'),

  assignList: (body: AssignBody) =>
    apiRequest<AssignResponse>('/api/assignments', { method: 'PUT', json: body }),

  blockDomain: (body: BlockBody) =>
    apiRequest<BlockResponse>('/api/rules/block', { method: 'POST', json: body }),

  createClient: (body: ClientUpdateBody) =>
    apiRequest<OkResponse>('/api/clients', { method: 'POST', json: body }),

  updateClient: (name: string, body: ClientUpdateBody) =>
    apiRequest<OkResponse>(`/api/clients/${encodeURIComponent(name)}`, { method: 'PUT', json: body }),

  deleteClient: (name: string) =>
    apiRequest<OkResponse>(`/api/clients/${encodeURIComponent(name)}`, { method: 'DELETE' }),

  createGroup: (body: GroupCreateBody) =>
    apiRequest<OkResponse>('/api/upstream-groups', { method: 'POST', json: body }),

  updateGroup: (name: string, body: GroupUpdateBody) =>
    apiRequest<OkResponse>(`/api/upstream-groups/${encodeURIComponent(name)}`, { method: 'PUT', json: body }),

  deleteGroup: (name: string) =>
    apiRequest<OkResponse>(`/api/upstream-groups/${encodeURIComponent(name)}`, { method: 'DELETE' }),

  createResolver: (body: ResolverBody) =>
    apiRequest<OkResponse>('/api/resolvers', { method: 'POST', json: body }),

  updateResolver: (name: string, body: ResolverBody) =>
    apiRequest<OkResponse>(`/api/resolvers/${encodeURIComponent(name)}`, { method: 'PUT', json: body }),

  deleteResolver: (name: string) =>
    apiRequest<OkResponse>(`/api/resolvers/${encodeURIComponent(name)}`, { method: 'DELETE' }),

  health: () => apiRequest<Health>('/api/health', { skipUnauthorized: true }),
};
