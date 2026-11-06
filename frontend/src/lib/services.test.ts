import { describe, expect, it } from 'vitest';
import type { ServiceItem } from '../api/types';
import { groupLabel, groupServices } from './services';

const items: ServiceItem[] = [
  { id: 'youtube', name: 'YouTube', group: 'streaming', domains: 3 },
  { id: 'netflix', name: 'Netflix', group: 'streaming', domains: 4 },
  { id: 'discord', name: 'Discord', group: 'messenger', domains: 5 },
  { id: 'tiktok', name: 'TikTok', group: 'social_network', domains: 6 },
];

describe('groupServices', () => {
  it('groups by category, sorted by label and then by name', () => {
    const groups = groupServices(items, '');
    expect(groups.map((g) => g.label)).toEqual(['Messengers', 'Social networks', 'Streaming']);
    expect(groups[2].services.map((s) => s.name)).toEqual(['Netflix', 'YouTube']);
  });

  it('finds an app by part of its name, ignoring case, and drops groups with no match', () => {
    const groups = groupServices(items, '  TUBE ');
    expect(groups).toHaveLength(1);
    expect(groups[0].services.map((s) => s.id)).toEqual(['youtube']);
  });

  it('returns nothing when no app matches, so the dialog can say so', () => {
    expect(groupServices(items, 'nope')).toEqual([]);
  });
});

describe('groupLabel', () => {
  it('names known groups and still shows a group the catalogue adds later', () => {
    expect(groupLabel('social_network')).toBe('Social networks');
    expect(groupLabel('new_thing')).toBe('New thing');
  });
});
