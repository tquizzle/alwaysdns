import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { ApiError } from './client';
import { api } from './endpoints';
import type {
  AssignBody,
  BlockBody,
  ClientUpdateBody,
  GroupCreateBody,
  GroupUpdateBody,
  HistoryParams,
  ListCreateBody,
  ListUpdateBody,
  Me,
  QueryLogParams,
  RangeKey,
} from './types';

export const POLL_STATS_MS = 5_000;
export const POLL_QUERYLOG_MS = 3_000;
export const POLL_CLIENTS_MS = 30_000;
export const POLL_HEALTH_MS = 15_000;

export const queryKeys = {
  me: ['me'] as const,
  stats: (range: RangeKey) => ['stats', range] as const,
  querylog: (params: QueryLogParams) => ['querylog', params] as const,
  history: (params: HistoryParams | null) => ['history', params] as const,
  lists: ['lists'] as const,
  clients: ['clients'] as const,
  health: ['health'] as const,
};

/** Resolves to null when there is no session (401), so the app can show Login. */
export function useMe() {
  return useQuery<Me | null>({
    queryKey: queryKeys.me,
    queryFn: async () => {
      try {
        return await api.me();
      } catch (err) {
        if (err instanceof ApiError && err.status === 401) return null;
        throw err;
      }
    },
    staleTime: 60_000,
    retry: false,
  });
}

export function useLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.login,
    onSuccess: (me) => {
      queryClient.setQueryData(queryKeys.me, me);
    },
  });
}

export function useLogout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.logout,
    onSettled: () => {
      queryClient.removeQueries({ predicate: (q) => q.queryKey[0] !== 'me' });
      queryClient.setQueryData(queryKeys.me, null);
    },
  });
}

export function useStats(range: RangeKey) {
  return useQuery({
    queryKey: queryKeys.stats(range),
    queryFn: () => api.stats(range),
    refetchInterval: POLL_STATS_MS,
    placeholderData: keepPreviousData,
  });
}

/**
 * Query log page. The caller polls only on the first page with no offset by
 * passing live=false for other pages. `enabled` is false while the filters are
 * invalid, so no request is sent.
 */
export function useQueryLog(params: QueryLogParams, live: boolean, enabled = true) {
  return useQuery({
    queryKey: queryKeys.querylog(params),
    queryFn: () => api.queryLog(params),
    enabled,
    refetchInterval: live ? POLL_QUERYLOG_MS : false,
    placeholderData: keepPreviousData,
  });
}

/** Timeline history for a chosen window. `null` disables the query. */
export function useHistory(params: HistoryParams | null) {
  return useQuery({
    queryKey: queryKeys.history(params),
    queryFn: () => api.history(params as HistoryParams),
    enabled: params !== null,
  });
}

export function useLists() {
  return useQuery({
    queryKey: queryKeys.lists,
    queryFn: api.lists,
  });
}

function useInvalidateLists() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: queryKeys.lists });
}

export function useCreateList() {
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: (body: ListCreateBody) => api.createList(body),
    onSuccess: () => invalidate(),
  });
}

export function useUpdateList() {
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: ({ id, body }: { id: number; body: ListUpdateBody }) => api.updateList(id, body),
    onSuccess: () => invalidate(),
  });
}

export function useDeleteList() {
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: (id: number) => api.deleteList(id),
    onSuccess: () => invalidate(),
  });
}

export function useRefreshLists() {
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: () => api.refreshLists(),
    onSuccess: () => invalidate(),
  });
}

export function useAssignList() {
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: (body: AssignBody) => api.assignList(body),
    onSuccess: () => invalidate(),
  });
}

export function useBlockDomain() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: BlockBody) => api.blockDomain(body),
    onSuccess: () => {
      // The block changes what the log and the dashboard show next.
      queryClient.invalidateQueries({ queryKey: ['querylog'] });
      queryClient.invalidateQueries({ queryKey: ['stats'] });
    },
  });
}

export function useClients() {
  return useQuery({
    queryKey: queryKeys.clients,
    queryFn: api.clients,
    refetchInterval: POLL_CLIENTS_MS,
  });
}

/** Client changes also move the list page's "Assigned to" chips, so both caches refresh. */
function useInvalidateClientViews() {
  const queryClient = useQueryClient();
  return () => {
    queryClient.invalidateQueries({ queryKey: queryKeys.clients });
    queryClient.invalidateQueries({ queryKey: queryKeys.lists });
  };
}

export function useUpdateClient() {
  const invalidate = useInvalidateClientViews();
  return useMutation({
    mutationFn: ({ name, body }: { name: string; body: ClientUpdateBody }) => api.updateClient(name, body),
    onSuccess: () => invalidate(),
  });
}

export function useCreateClient() {
  const invalidate = useInvalidateClientViews();
  return useMutation({
    mutationFn: (body: ClientUpdateBody) => api.createClient(body),
    onSuccess: () => invalidate(),
  });
}

export function useDeleteClient() {
  const invalidate = useInvalidateClientViews();
  return useMutation({
    mutationFn: (name: string) => api.deleteClient(name),
    onSuccess: () => invalidate(),
  });
}

export function useCreateGroup() {
  const invalidate = useInvalidateClientViews();
  return useMutation({
    mutationFn: (body: GroupCreateBody) => api.createGroup(body),
    onSuccess: () => invalidate(),
  });
}

export function useUpdateGroup() {
  const invalidate = useInvalidateClientViews();
  return useMutation({
    mutationFn: ({ name, body }: { name: string; body: GroupUpdateBody }) => api.updateGroup(name, body),
    onSuccess: () => invalidate(),
  });
}

export function useDeleteGroup() {
  const invalidate = useInvalidateClientViews();
  return useMutation({
    mutationFn: (name: string) => api.deleteGroup(name),
    onSuccess: () => invalidate(),
  });
}

export function useHealth() {
  return useQuery({
    queryKey: queryKeys.health,
    queryFn: api.health,
    refetchInterval: POLL_HEALTH_MS,
    retry: 1,
  });
}
