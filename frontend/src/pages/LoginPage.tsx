import { useState, type FormEvent } from 'react';
import { ApiError } from '../api/client';
import { useLogin } from '../api/queries';

export function LoginPage() {
  const login = useLogin();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!username.trim() || !password) return;
    login.mutate({ username: username.trim(), password });
  };

  let errorText: string | null = null;
  if (login.error) {
    errorText =
      login.error instanceof ApiError && login.error.status === 401
        ? 'Wrong username or password.'
        : login.error.message;
  }

  return (
    <main className="login-screen">
      <form className="panel login-card" onSubmit={onSubmit} noValidate>
        <div className="brand brand-lg">
          <BrandMark />
          <span>AlwaysDNS</span>
        </div>
        <p className="muted">Sign in to view queries, clients and settings.</p>

        <label className="field">
          <span>Username</span>
          <input
            name="username"
            autoComplete="username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoFocus
            required
          />
        </label>

        <label className="field">
          <span>Password</span>
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>

        {errorText && (
          <div className="state state-error" role="alert">
            {errorText}
          </div>
        )}

        <button type="submit" className="btn btn-primary" disabled={login.isPending}>
          {login.isPending ? 'Signing in...' : 'Sign in'}
        </button>
      </form>
    </main>
  );
}

export function BrandMark() {
  return (
    <svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
      <circle cx="16" cy="16" r="12" fill="none" stroke="currentColor" strokeWidth="3" strokeDasharray="60 20" transform="rotate(-90 16 16)" />
      <circle cx="16" cy="16" r="3.5" fill="currentColor" />
    </svg>
  );
}
