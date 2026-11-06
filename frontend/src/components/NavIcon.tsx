import type { ReactNode } from 'react';
import type { Route } from '../lib/hashRoute';

/** Inner shapes of each nav icon, drawn on a 20 unit grid with the same stroke as the rings. */
const SHAPES: Record<Route, ReactNode> = {
  settings: (
    <>
      <path d="M3.5 6h13M3.5 10h13M3.5 14h13" />
      <circle cx="7" cy="6" r="1.7" />
      <circle cx="13" cy="10" r="1.7" />
      <circle cx="8.5" cy="14" r="1.7" />
    </>
  ),
  dashboard: (
    <>
      <path d="M3.5 13a6.5 6.5 0 0 1 13 0" />
      <path d="M10 13l3-3" />
    </>
  ),
  querylog: (
    <>
      <rect x="3.5" y="3.5" width="13" height="13" rx="2" />
      <path d="M6.5 7h7M6.5 10h7M6.5 13h4" />
    </>
  ),
  lists: (
    <>
      <circle cx="5" cy="5" r="0.75" fill="currentColor" />
      <circle cx="5" cy="10" r="0.75" fill="currentColor" />
      <circle cx="5" cy="15" r="0.75" fill="currentColor" />
      <path d="M8.5 5h8M8.5 10h8M8.5 15h8" />
    </>
  ),
  upstreams: (
    <>
      <circle cx="5" cy="4.5" r="1.5" />
      <circle cx="15" cy="4.5" r="1.5" />
      <circle cx="10" cy="15.5" r="1.5" />
      <path d="M5 6v1.5L10 11.5 15 7.5V6M10 11.5V14" />
    </>
  ),
  clients: (
    <>
      <rect x="3" y="4" width="14" height="9.5" rx="1.5" />
      <path d="M7.5 16.5h5M10 13.5v3" />
    </>
  ),
};

/** Decorative icon beside a nav label. The label is the accessible name. */
export function NavIcon({ route }: { route: Route }) {
  return (
    <svg
      className="nav-icon"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {SHAPES[route]}
    </svg>
  );
}
