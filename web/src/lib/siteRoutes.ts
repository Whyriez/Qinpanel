import type { Site } from "../types/api";

export function sitePath(
  site: Pick<Site, "primary_domain">,
  tab = "overview",
): string {
  return `/sites/${encodeURIComponent(site.primary_domain.toLowerCase())}/${tab}`;
}

export function parseSiteRoute(
  pathname: string,
): { key: string; suffix: string } | null {
  const match = /^\/sites\/([^/]+)(\/.*)?$/.exec(pathname);
  if (!match) return null;
  try {
    const key = decodeURIComponent(match[1]);
    if (
      !key ||
      key === "." ||
      key === ".." ||
      /[/\\\s?#]/.test(key) ||
      Array.from(key).some((character) => character.charCodeAt(0) < 32)
    )
      return null;
    return {
      key,
      suffix: match[2] && match[2] !== "/" ? match[2] : "/overview",
    };
  } catch {
    return null;
  }
}

export function canonicalSiteUrl(
  site: Pick<Site, "primary_domain">,
  pathname: string,
  search = "",
  hash = "",
): string {
  const suffix = parseSiteRoute(pathname)?.suffix || "/overview";
  return sitePath(site, suffix.slice(1)) + search + hash;
}
