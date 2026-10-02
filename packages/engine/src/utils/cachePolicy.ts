/**
 * Whether the proxies strip cache validators from the requests they forward ("Disable cache" in the capture
 * panel). A browser that holds a copy sends `If-None-Match` / `If-Modified-Since`, the server answers `304`
 * with no body, and the capture has nothing to show. On by default; held in memory, so an engine restart
 * turns it back on.
 */
let disabled = true;

export function setCacheDisabled(value: boolean): void {
  disabled = value;
}

export function isCacheDisabled(): boolean {
  return disabled;
}

/** The request headers that make a server answer "not modified" or a cache serve a stored copy. */
const CACHE_REQUEST_HEADERS = ["if-none-match", "if-modified-since", "cache-control", "pragma"];

/** Removes the cache headers from `headers` (lower-case keys) in place, when the policy asks for it. */
export function stripCacheHeaders(headers: Record<string, string>): void {
  if (!disabled) return;
  for (const name of CACHE_REQUEST_HEADERS) delete headers[name];
}
