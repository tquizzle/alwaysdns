import type { ServiceItem } from '../api/types';

const GROUP_LABELS: Record<string, string> = {
  ai: 'AI',
  cdn: 'CDNs',
  dating: 'Dating',
  gambling: 'Gambling',
  gaming: 'Gaming',
  hosting: 'Hosting',
  messenger: 'Messengers',
  privacy: 'Privacy tools',
  shopping: 'Shopping',
  social_network: 'Social networks',
  software: 'Software',
  streaming: 'Streaming',
};

/** A readable group name. A group the catalogue adds later still shows, as its id with spaces. */
export function groupLabel(group: string): string {
  const known = GROUP_LABELS[group];
  if (known) return known;
  const spaced = group.replace(/_/g, ' ');
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

export interface ServiceGroup {
  group: string;
  label: string;
  services: ServiceItem[];
}

/** Apps whose name or id contains the query, grouped by category. Groups and apps are sorted by name. */
export function groupServices(services: ServiceItem[], query: string): ServiceGroup[] {
  const needle = query.trim().toLowerCase();
  const byGroup = new Map<string, ServiceItem[]>();
  for (const service of services) {
    if (needle && !service.name.toLowerCase().includes(needle) && !service.id.includes(needle)) continue;
    byGroup.set(service.group, [...(byGroup.get(service.group) ?? []), service]);
  }
  return [...byGroup.entries()]
    .map(([group, items]) => ({
      group,
      label: groupLabel(group),
      services: [...items].sort((a, b) => a.name.localeCompare(b.name)),
    }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
