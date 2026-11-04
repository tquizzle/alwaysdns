import type { NameCount } from '../api/types';
import { formatInt } from '../lib/format';
import world from '../assets/world-paths.json';

const WORLD = world as { width: number; height: number; countries: Record<string, string> };

const LOW: [number, number, number] = [30, 41, 59]; // slate: few queries
const HIGH: [number, number, number] = [34, 211, 238]; // cyan: most queries

function shade(t: number): string {
  const mix = (a: number, b: number) => Math.round(a + (b - a) * t);
  return `rgb(${mix(LOW[0], HIGH[0])}, ${mix(LOW[1], HIGH[1])}, ${mix(LOW[2], HIGH[2])})`;
}

interface Props {
  /** Query counts per destination country, named by ISO code. */
  countries: NameCount[];
  /** False when the server has no GeoLite2 database loaded. */
  geoReady: boolean;
}

/**
 * Traffic destination: each country shaded by how many answers pointed there.
 * Countries with no queries stay in the base colour.
 */
export function TrafficMap({ countries, geoReady }: Props) {
  if (!geoReady) {
    return (
      <div className="state state-empty">
        Destination map is off. Place a MaxMind <code>GeoLite2-Country.mmdb</code> file in the
        server's data folder, then restart the container.
      </div>
    );
  }

  const byCode = new Map(countries.map((c) => [c.name, c.count]));
  const max = Math.max(1, ...countries.map((c) => c.count));
  // Square root keeps small countries visible next to the very large ones.
  const shadeFor = (count: number) => (count > 0 ? shade(Math.sqrt(count / max)) : shade(0));

  return (
    <div className="traffic-map">
      <svg
        viewBox={`0 0 ${WORLD.width} ${WORLD.height}`}
        role="img"
        aria-label="Destination countries of DNS answers"
      >
        {Object.entries(WORLD.countries).map(([code, d]) => {
          const count = byCode.get(code) ?? 0;
          return (
            <path key={code} d={d} fill={shadeFor(count)} className="country">
              <title>{`${code}: ${formatInt(count)} queries`}</title>
            </path>
          );
        })}
      </svg>
      <p className="attribution small muted">
        This product includes GeoLite2 data created by MaxMind, available from{' '}
        <a href="https://www.maxmind.com" target="_blank" rel="noreferrer">
          maxmind.com
        </a>
        .
      </p>
    </div>
  );
}
