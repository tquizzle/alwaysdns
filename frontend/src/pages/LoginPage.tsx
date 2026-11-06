import { useId, useState, type FormEvent } from 'react';
import { ApiError } from '../api/client';
import { useLogin } from '../api/queries';
import { Footer } from '../components/Footer';

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
          <Wordmark />
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
      <Footer />
    </main>
  );
}

/** Tick marks around the orb, longer every third one, like the dashboard gauge. */
const TICKS = Array.from({ length: 12 }, (_, i) => {
  const angle = (i / 12) * 2 * Math.PI;
  const inner = i % 3 === 0 ? 12.6 : 13.6;
  return {
    i,
    x1: 16 + Math.cos(angle) * inner,
    y1: 16 + Math.sin(angle) * inner,
    x2: 16 + Math.cos(angle) * 15,
    y2: 16 + Math.sin(angle) * 15,
    long: i % 3 === 0,
  };
});

/** A small orb: a glowing ring with a gap, and a red dot in the gap. The red dot is the outage. */
export function BrandMark() {
  const glowId = useId().replace(/:/g, '');
  return (
    <svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <radialGradient id={glowId}>
          <stop offset="0" stopColor="currentColor" stopOpacity="0.3" />
          <stop offset="1" stopColor="currentColor" stopOpacity="0" />
        </radialGradient>
      </defs>
      <circle cx="16" cy="16" r="15.5" fill={`url(#${glowId})`} />
      {TICKS.map((t) => (
        <line
          key={t.i}
          x1={t.x1.toFixed(2)}
          y1={t.y1.toFixed(2)}
          x2={t.x2.toFixed(2)}
          y2={t.y2.toFixed(2)}
          stroke="currentColor"
          strokeOpacity={t.long ? 0.85 : 0.4}
          strokeWidth="0.9"
        />
      ))}
      <circle cx="16" cy="16" r="9" fill="none" stroke="currentColor" strokeWidth="3.4" strokeLinecap="round" strokeDasharray="41 15.5" />
      <circle cx="16" cy="16" r="2.6" fill="currentColor" />
      <circle cx="21.8" cy="9.2" r="2.2" style={{ fill: 'var(--red)' }} />
    </svg>
  );
}

/** "AlwaysDNS" with DNS in the accent colour. Reads as one word to a screen reader. */
export function Wordmark() {
  return (
    <span>
      Always<span className="brand-dns">DNS</span>
    </span>
  );
}
