import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { readStorage, writeStorage } from './storage';
import { DEFAULT_PALETTE, PALETTES, applyPalette, isPaletteId, type PaletteId } from './palettes';

const REDACT_KEY = 'dnsui.redactAddresses';
const PALETTE_KEY = 'dnsui.palette';

interface Prefs {
  redactAddresses: boolean;
  setRedactAddresses: (value: boolean) => void;
  palette: PaletteId;
  setPalette: (value: PaletteId) => void;
}

const PrefsContext = createContext<Prefs | null>(null);

export function PrefsProvider({ children }: { children: ReactNode }) {
  const [redactAddresses, setRedactState] = useState(() => readStorage(REDACT_KEY, '0') === '1');
  const [palette, setPaletteState] = useState<PaletteId>(() => {
    const stored = readStorage(PALETTE_KEY, DEFAULT_PALETTE);
    return isPaletteId(stored) ? stored : DEFAULT_PALETTE;
  });

  const setRedactAddresses = useCallback((value: boolean) => {
    setRedactState(value);
    writeStorage(REDACT_KEY, value ? '1' : '0');
  }, []);

  const setPalette = useCallback((value: PaletteId) => {
    setPaletteState(value);
    writeStorage(PALETTE_KEY, value);
  }, []);

  useEffect(() => {
    applyPalette(palette);
  }, [palette]);

  const value = useMemo(
    () => ({ redactAddresses, setRedactAddresses, palette, setPalette }),
    [redactAddresses, setRedactAddresses, palette, setPalette],
  );

  return <PrefsContext.Provider value={value}>{children}</PrefsContext.Provider>;
}

export function usePrefs(): Prefs {
  const ctx = useContext(PrefsContext);
  if (!ctx) throw new Error('usePrefs must be used inside PrefsProvider');
  return ctx;
}

/** The accent colour for SVG charts. Falls back to the default palette outside the provider. */
export function useAccent(): string {
  const ctx = useContext(PrefsContext);
  return PALETTES[ctx?.palette ?? DEFAULT_PALETTE].accent;
}
