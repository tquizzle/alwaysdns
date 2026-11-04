// Thin typed wrapper around fetch. All requests go to same-origin /api paths.

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

type UnauthorizedHandler = () => void;
let onUnauthorized: UnauthorizedHandler | null = null;

/** Registered once at startup. Called whenever a protected request returns 401. */
export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  onUnauthorized = handler;
}

export interface RequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
  json?: unknown;
  /** Login and session checks handle 401 themselves, so skip the global handler. */
  skipUnauthorized?: boolean;
  signal?: AbortSignal;
}

function messageFromBody(text: string): string {
  if (!text) return '';
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed && typeof parsed === 'object') {
      const obj = parsed as Record<string, unknown>;
      const msg = obj.error ?? obj.message;
      if (typeof msg === 'string') return msg;
    }
  } catch {
    // Not JSON; fall through to the plain text below.
  }
  return text.length <= 200 ? text : '';
}

export async function apiRequest<T>(path: string, opts: RequestOptions = {}): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' };
  let body: string | undefined;
  if (opts.json !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(opts.json);
  }

  let res: Response;
  try {
    res = await fetch(path, {
      method: opts.method ?? (body === undefined ? 'GET' : 'POST'),
      headers,
      body,
      credentials: 'same-origin',
      signal: opts.signal,
    });
  } catch (err) {
    if (err instanceof DOMException && err.name === 'AbortError') throw err;
    throw new ApiError(0, 'Cannot reach the DNS server. Check that it is running.');
  }

  if (res.status === 401 && !opts.skipUnauthorized) {
    onUnauthorized?.();
  }

  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new ApiError(res.status, messageFromBody(text) || `Request failed (HTTP ${res.status})`);
  }

  if (res.status === 204) return undefined as T;

  const contentType = res.headers.get('content-type') ?? '';
  if (contentType.includes('application/json')) {
    return (await res.json()) as T;
  }
  return (await res.text()) as unknown as T;
}
