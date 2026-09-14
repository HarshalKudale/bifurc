import * as fs from "fs";
import { createCert } from "mkcert";

interface CertKeyPair {
  cert: string;
  key: string;
}

// CA stored as raw PEM strings — no forge parsing required
let caCertPem: string | null = null;
let caKeyPem: string | null = null;

// Cache Promises so parallel requests for the same host share one generation run
const certCache = new Map<string, Promise<CertKeyPair>>();
const MAX_CACHE_SIZE = 500;

export function loadCA(certPath: string, keyPath: string): boolean {
  try {
    caCertPem = fs.readFileSync(certPath, "utf-8");
    caKeyPem = fs.readFileSync(keyPath, "utf-8");
    clearCertCache();
    return true;
  } catch {
    caCertPem = null;
    caKeyPem = null;
    return false;
  }
}

export function unloadCA(): void {
  caCertPem = null;
  caKeyPem = null;
  clearCertCache();
}

export function isCALoaded(): boolean {
  return caCertPem !== null && caKeyPem !== null;
}

export function clearCertCache(): void {
  certCache.clear();
}

export function generateHostCert(hostname: string): Promise<CertKeyPair> {
  const cached = certCache.get(hostname);
  if (cached) return cached;

  if (!caCertPem || !caKeyPem) return Promise.reject(new Error("CA not loaded"));

  const promise = createCert({
    domains: [hostname],
    validity: 365,
    ca: { cert: caCertPem, key: caKeyPem },
  });

  // Evict oldest entry if at capacity
  if (certCache.size >= MAX_CACHE_SIZE) {
    const firstKey = certCache.keys().next().value;
    if (firstKey !== undefined) certCache.delete(firstKey);
  }
  certCache.set(hostname, promise);

  // Remove from cache on failure so the next attempt retries
  promise.catch(() => certCache.delete(hostname));

  return promise;
}
