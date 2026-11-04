/**
 * Colour palettes. Each one sets the page background, panel colours and the
 * accent colour. Status colours (red, yellow, green) stay fixed in every
 * palette, so a blocked query always reads as red.
 */
export interface Palette {
  label: string;
  bgTop: string;
  bgBottom: string;
  panel: string;
  panelSolid: string;
  /** Rgb triplet used for borders and faint fills, e.g. "31, 182, 255". */
  edgeRgb: string;
  /** Rgb triplet of the accent, for translucent fills. */
  accentRgb: string;
  accent: string;
  accentStrong: string;
  accentSoft: string;
}

export const PALETTES = {
  midnight: {
    label: 'Midnight cyan',
    bgTop: '#050e1f',
    bgBottom: '#0a1a33',
    panel: 'rgba(10, 30, 60, 0.6)',
    panelSolid: '#0b1d38',
    edgeRgb: '31, 182, 255',
    accentRgb: '34, 211, 238',
    accent: '#22d3ee',
    accentStrong: '#0ea5c6',
    accentSoft: '#67e8f9',
  },
  violet: {
    label: 'Violet night',
    bgTop: '#0a0716',
    bgBottom: '#161033',
    panel: 'rgba(30, 20, 62, 0.6)',
    panelSolid: '#1a1338',
    edgeRgb: '167, 139, 250',
    accentRgb: '167, 139, 250',
    accent: '#a78bfa',
    accentStrong: '#7c3aed',
    accentSoft: '#c4b5fd',
  },
  emerald: {
    label: 'Emerald forest',
    bgTop: '#03120c',
    bgBottom: '#0a2118',
    panel: 'rgba(10, 45, 30, 0.6)',
    panelSolid: '#0b2a1d',
    edgeRgb: '52, 211, 153',
    accentRgb: '52, 211, 153',
    accent: '#34d399',
    accentStrong: '#059669',
    accentSoft: '#6ee7b7',
  },
  amber: {
    label: 'Amber dusk',
    bgTop: '#120c03',
    bgBottom: '#211606',
    panel: 'rgba(50, 35, 10, 0.6)',
    panelSolid: '#2a1d0b',
    edgeRgb: '251, 191, 36',
    accentRgb: '251, 191, 36',
    accent: '#fbbf24',
    accentStrong: '#d97706',
    accentSoft: '#fde68a',
  },
} as const satisfies Record<string, Palette>;

export type PaletteId = keyof typeof PALETTES;

export const PALETTE_IDS = Object.keys(PALETTES) as PaletteId[];

export const DEFAULT_PALETTE: PaletteId = 'midnight';

export function isPaletteId(value: string): value is PaletteId {
  return (PALETTE_IDS as string[]).includes(value);
}

/** Writes a palette onto the document root as CSS variables. */
export function applyPalette(id: PaletteId): void {
  const p: Palette = PALETTES[id];
  const style = document.documentElement.style;
  style.setProperty('--bg-top', p.bgTop);
  style.setProperty('--bg-bottom', p.bgBottom);
  style.setProperty('--panel', p.panel);
  style.setProperty('--panel-solid', p.panelSolid);
  style.setProperty('--edge-rgb', p.edgeRgb);
  style.setProperty('--accent-rgb', p.accentRgb);
  style.setProperty('--accent', p.accent);
  style.setProperty('--accent-strong', p.accentStrong);
  style.setProperty('--accent-soft', p.accentSoft);
  style.setProperty('--cyan', p.accent);
}
