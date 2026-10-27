function normalize(value: string): string {
  return value.normalize('NFD').replace(/\p{M}/gu, '').toLocaleLowerCase().trim();
}

/** Match actual menu words so unrelated fuzzy matches cannot outrank an action. */
export function filterMenuCommands(value: string, search: string, keywords: string[] = []): number {
  const query = normalize(search);
  if (!query) return 1;
  const labels = [value, ...keywords].map(normalize);
  if (labels.includes(query)) return 1;
  if (labels.some(label => label.startsWith(query))) return 0.9;
  const haystack = labels.join(' ');
  return query.split(/\s+/).every(word => haystack.includes(word)) ? 0.7 : 0;
}
