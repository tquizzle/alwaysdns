import type { Rewrite, Settings, SettingsBody } from '../api/types';

/** The body for a save: everything the server has now, with one section changed. */
export function settingsBody(current: Settings, patch: Partial<SettingsBody>): SettingsBody {
  return {
    rewrites: current.rewrites,
    allow_domains: current.allow_domains,
    block_domains: current.block_domains,
    bootstrap: current.bootstrap,
    default_upstream_group: current.default_upstream_group,
    retention: current.retention,
    ...patch,
  };
}

/** `alwaysdns-settings-2026-11-06.yaml`, so exports from different days do not overwrite each other. */
export function exportFilename(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `alwaysdns-settings-${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}.yaml`;
}

/** A whole number of days, at least 1, or null. */
export function parseDays(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,5}$/.test(trimmed)) return null;
  const days = Number(trimmed);
  return days >= 1 ? days : null;
}

/** A loose IPv4 or IPv6 check. The server does the real one. */
export function looksLikeIp(value: string): boolean {
  const text = value.trim();
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(text)) return text.split('.').every((part) => Number(part) <= 255);
  return text.includes(':') && /^[0-9a-fA-F:.]+$/.test(text);
}

/** Rows with both boxes empty are ignored, so an unused "Add" row does not block a save. */
export function nonBlankRows(rows: Rewrite[]): Rewrite[] {
  return rows.filter((row) => row.domain.trim() !== '' || row.answer.trim() !== '');
}

/** A message for a local answer that cannot be saved, or null when it looks fine. */
export function rewriteError(row: Rewrite): string | null {
  if (row.domain.trim() === '') return 'Give each local answer a name, such as nas.example.test or *.example.test.';
  if (!looksLikeIp(row.answer)) return `"${row.answer.trim()}" is not an IP address.`;
  return null;
}
