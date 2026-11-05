import { useEffect, useId, useRef, useState } from 'react';
import { useLogout } from '../api/queries';
import { usePrefs } from '../lib/prefs';
import { PaletteSelect } from './PaletteSelect';

function RedactToggle() {
  const { redactAddresses, setRedactAddresses } = usePrefs();
  return (
    <button
      type="button"
      className="btn btn-small"
      aria-pressed={redactAddresses}
      title="Mask the last two octets of client addresses on screen"
      onClick={() => setRedactAddresses(!redactAddresses)}
    >
      {redactAddresses ? 'IPs masked' : 'Mask IPs'}
    </button>
  );
}

interface Props {
  username: string;
  role: string;
}

/** Account button that opens the display settings and Sign out, keeping the top bar short. */
export function UserMenu({ username, role }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();
  const logout = useLogout();

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', onPointer);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <div className="user-menu" ref={rootRef}>
      <button
        ref={buttonRef}
        type="button"
        className="btn btn-small user-menu-btn"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        <span>{username}</span>
        <small>{role}</small>
        <span aria-hidden="true">▾</span>
      </button>
      {open && (
        <div id={menuId} className="menu-panel panel" role="group" aria-label="Account and display">
          <p className="muted small">Signed in as {username} ({role})</p>
          <RedactToggle />
          <label className="field">
            <span>Colour palette</span>
            <PaletteSelect />
          </label>
          <button type="button" className="btn btn-small" disabled={logout.isPending} onClick={() => logout.mutate()}>
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}
