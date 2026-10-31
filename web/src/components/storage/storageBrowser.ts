import { fetchStorageObjects, validObjectKey, type StorageConnection, type StorageListing, type StorageObject } from '../../api/storage';

export function storageFolderKey(prefix: string, name: string): string | null {
  if (!name.trim() || name.includes('/') || name.includes('\\') || name === '.' || name === '..' || prefix && !prefix.endsWith('/')) return null;
  const key = `${prefix}${name}/`;
  return validObjectKey(key) ? key : null;
}
export const isFolderMarker = (object: StorageObject) => object.size === 0 && object.key.endsWith('/');

export interface StorageSearchResult { listing: StorageListing; scanned: number; partial: boolean }
/** Owner-authorized provider pages only. Tokens stay opaque and requests are cancellable. */
export async function searchStorageFiles(connection: StorageConnection, prefix: string, query: string, recursive: boolean, signal: AbortSignal): Promise<StorageSearchResult> {
  const queue: { prefix: string; token: string | null }[] = [{ prefix, token: null }];
  const seenPages = new Set<string>(), seenFolders = new Set([prefix]);
  const objects = new Map<string, StorageObject>(), folders = new Set<string>();
  const needle = query.trim().toLocaleLowerCase();
  let scanned = 0, partial = false;
  const matches = (key: string) => key.slice(prefix.length).toLocaleLowerCase().includes(needle);
  while (queue.length && seenPages.size < 40 && scanned < 5000 && objects.size + folders.size < 250) {
    if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
    const next = queue.shift()!;
    const cursor = JSON.stringify([next.prefix, next.token]);
    if (seenPages.has(cursor)) { partial = true; continue; }
    seenPages.add(cursor);
    const { listing } = await fetchStorageObjects(connection, next.prefix, next.token, signal);
    const addFolder = (child: string) => {
      if (!child.startsWith(next.prefix) || child === next.prefix || !child.endsWith('/')) return;
      if (matches(child)) {
        if (folders.size + objects.size < 250) folders.add(child); else partial = true;
      }
      if (recursive && !seenFolders.has(child)) {
        seenFolders.add(child);
        if (seenFolders.size <= 500) queue.push({ prefix: child, token: null }); else partial = true;
      }
    };
    for (const object of listing.objects) {
      scanned++;
      if (isFolderMarker(object)) addFolder(object.key);
      else if (object.key.startsWith(prefix) && matches(object.key) && objects.size + folders.size < 250) objects.set(object.key, object);
      if (scanned >= 5000 || objects.size + folders.size >= 250) { partial = true; break; }
    }
    listing.prefixes.forEach(addFolder);
    if (listing.next_token) queue.push({ prefix: next.prefix, token: listing.next_token });
  }
  return { listing: { objects: [...objects.values()], prefixes: [...folders], next_token: null }, scanned, partial: partial || queue.length > 0 };
}
