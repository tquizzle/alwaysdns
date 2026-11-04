import { PALETTE_IDS, PALETTES, isPaletteId } from '../lib/palettes';
import { usePrefs } from '../lib/prefs';

/** Picks the colour palette for this browser. Stored like the Mask IPs setting. */
export function PaletteSelect() {
  const { palette, setPalette } = usePrefs();
  return (
    <select
      className="btn btn-small palette-picker"
      aria-label="Colour palette"
      title="Colour palette, saved in this browser"
      value={palette}
      onChange={(e) => {
        const next = e.target.value;
        if (isPaletteId(next)) setPalette(next);
      }}
    >
      {PALETTE_IDS.map((id) => (
        <option key={id} value={id}>
          {PALETTES[id].label}
        </option>
      ))}
    </select>
  );
}
