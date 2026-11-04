import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { App } from './App';
import { ApiError, setUnauthorizedHandler } from './api/client';
import { queryKeys } from './api/queries';
import { ErrorBoundary } from './components/ErrorBoundary';
import { PrefsProvider } from './lib/prefs';
import './styles.css';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Do not retry client errors such as 401 or 404; retry transient failures twice.
      retry: (failureCount, error) => {
        if (error instanceof ApiError && error.status >= 400 && error.status < 500) return false;
        return failureCount < 2;
      },
      refetchOnWindowFocus: true,
    },
  },
});

// Any protected request that returns 401 drops the session, which shows the login page.
setUnauthorizedHandler(() => {
  queryClient.setQueryData(queryKeys.me, null);
});

const root = document.getElementById('root');
if (!root) throw new Error('Missing #root element');

createRoot(root).render(
  <StrictMode>
    <ErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <PrefsProvider>
          <App />
        </PrefsProvider>
      </QueryClientProvider>
    </ErrorBoundary>
  </StrictMode>,
);
