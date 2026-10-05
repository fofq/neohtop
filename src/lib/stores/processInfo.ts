import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { Process, ProcessMetadata, ServiceInfo } from "$lib/types";

/**
 * Caches for the details modal and the row hover card: process metadata is
 * keyed by executable path (shared by every PID running the same exe), the
 * service list is loaded once and reused for both the services tab and the
 * "hosted services" line of the hover card.
 */

/** Metadata cache keyed by lowercased executable path. */
export const metadataCache = writable<Map<string, ProcessMetadata>>(new Map());

// pid -> exe path filled in from backend responses, so later hovers of the
// same PID hit the cache even when no path could be guessed up front
const pidPaths = new Map<number, string>();
// Requests currently running, keyed by exe path hint or PID, so concurrent
// hovers of the same executable share one call
const metadataInFlight = new Map<string, Promise<ProcessMetadata | null>>();

/**
 * Best-effort executable path guessed from the command line, used as a
 * cache key before the backend has answered; svchost instances share one
 * exe path, so this collapses their lookups into a single request.
 */
function exePathHint(process: Process): string | null {
  const command = process.command.trim();
  if (!command) return null;
  const quoted = command.startsWith('"')
    ? command.slice(1, command.indexOf('"', 1))
    : command.split(" ")[0];
  return quoted && /[\\/]/.test(quoted) ? quoted.toLowerCase() : null;
}

/**
 * Cache-only lookup used by reactive components (the hover card): mirrors
 * the keys ensureProcessMetadata fills in, without triggering a request.
 */
export function lookupProcessMetadata(
  process: Process,
): ProcessMetadata | null {
  const cache = get(metadataCache);
  const hint = exePathHint(process);
  if (hint && cache.has(hint)) {
    return cache.get(hint) ?? null;
  }
  const path = pidPaths.get(process.pid);
  if (path && cache.has(path)) {
    return cache.get(path) ?? null;
  }
  return null;
}

/**
 * Fetches (or reuses) the metadata of a process. Resolves with the cached
 * or newly fetched metadata, or null when the backend refused (e.g. a
 * protected process); failures are not cached so a retry can succeed.
 */
export async function ensureProcessMetadata(
  process: Process,
): Promise<ProcessMetadata | null> {
  const cached = lookupProcessMetadata(process);
  if (cached) return cached;
  const hint = exePathHint(process);
  const key = hint ?? `pid:${process.pid}`;
  const pending = metadataInFlight.get(key);
  if (pending) return pending;
  const request = invoke<ProcessMetadata>("get_process_metadata", {
    pid: process.pid,
  })
    .then((metadata) => {
      const path = (metadata.exe_path || hint || "").toLowerCase();
      if (metadata.exe_path) pidPaths.set(process.pid, metadata.exe_path);
      if (path) {
        metadataCache.update((cache) => new Map(cache).set(path, metadata));
      }
      return metadata;
    })
    .catch((e) => {
      console.error("Failed to load process metadata:", e);
      return null;
    })
    .finally(() => {
      metadataInFlight.delete(key);
    });
  metadataInFlight.set(key, request);
  return request;
}

interface ServicesCache {
  /** null until the first successful load. */
  services: ServiceInfo[] | null;
  error: string | null;
  loading: boolean;
  /** Whether a load has completed (successfully or not). */
  loaded: boolean;
}

const servicesCache = writable<ServicesCache>({
  services: null,
  error: null,
  loading: false,
  loaded: false,
});

/**
 * Loads the service list once and caches it; `force` re-reads it (used by
 * the refresh button and after a service control action). Safe to call
 * repeatedly: concurrent calls share one request.
 */
export async function ensureServices(force = false): Promise<void> {
  const state = get(servicesCache);
  if (state.loading || (state.loaded && !force)) return;
  servicesCache.update((cache) => ({ ...cache, loading: true, error: null }));
  try {
    const services = await invoke<ServiceInfo[]>("list_services");
    servicesCache.set({
      services,
      error: null,
      loading: false,
      loaded: true,
    });
  } catch (e) {
    servicesCache.update((cache) => ({
      ...cache,
      error: e instanceof Error ? e.message : String(e),
      loading: false,
      loaded: true,
    }));
  }
}

/** Groups services by the PID of their hosting process (svchost shares). */
export function groupServicesByPid(
  services: ServiceInfo[],
): Map<number, ServiceInfo[]> {
  const groups = new Map<number, ServiceInfo[]>();
  for (const service of services) {
    if (service.pid === 0) continue;
    const group = groups.get(service.pid);
    if (group) {
      group.push(service);
    } else {
      groups.set(service.pid, [service]);
    }
  }
  return groups;
}

/**
 * Read helper for components that want the current cache value alongside
 * the store subscription.
 */
export const servicesCacheStore = servicesCache;
