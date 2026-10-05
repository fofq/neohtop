<script lang="ts">
  import { onDestroy } from "svelte";
  import { platform } from "@tauri-apps/plugin-os";
  import Fa from "svelte-fa";
  import { faUserShield } from "@fortawesome/free-solid-svg-icons";
  import { t } from "$lib/i18n";
  import {
    ensureProcessMetadata,
    ensureServices,
    lookupProcessMetadata,
    metadataCache,
    servicesCacheStore,
  } from "$lib/stores/index";
  import type { Process, ServiceInfo } from "$lib/types";

  /**
   * Lightweight hover card for process table rows: appears HOVER_DELAY_MS
   * after a row reports a hover and disappears on leave. Metadata is cached
   * by executable path (shared by every PID of the same exe) and the
   * service list is loaded once and shared with the services tab, so
   * hovering never requests more than one missing item per row.
   *
   * The parent drives it through bind:this + show()/hide().
   */

  const HOVER_DELAY_MS = 800;
  // Rough card size for keeping the popup inside the viewport
  const CARD_W = 320;
  const CARD_H = 190;

  let hovered: Process | null = null;
  let x = 0;
  let y = 0;
  let visible = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const isWindows = platform() === "windows";

  export function show(process: Process, px: number, py: number) {
    hovered = process;
    x = px;
    y = py;
    if (timer) clearTimeout(timer);
    visible = false;
    // Metadata prefetch starts right away so it is usually ready when the
    // card appears after the delay
    timer = setTimeout(() => {
      visible = true;
    }, HOVER_DELAY_MS);
  }

  export function hide() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    visible = false;
    hovered = null;
  }

  onDestroy(() => {
    if (timer) clearTimeout(timer);
  });

  $: if (hovered) {
    loadFor(hovered);
  }

  async function loadFor(process: Process) {
    await ensureProcessMetadata(process);
    // The hosted-services line only makes sense for shared service hosts
    if (isWindows && process.name.toLowerCase() === "svchost.exe") {
      ensureServices();
    }
  }

  // Recomputed when a hover starts AND whenever the metadata cache gains an
  // entry: the $metadataCache reference creates the store subscription the
  // cache-only lookup (via get() inside the store module) cannot provide
  $: metadata =
    hovered && $metadataCache ? lookupProcessMetadata(hovered) : null;

  function servicesFor(
    process: Process | null,
    services: ServiceInfo[] | null,
  ): ServiceInfo[] {
    if (!isWindows || !process || !services) return [];
    // The hosted-services line only makes sense for shared service hosts
    if (process.name.toLowerCase() !== "svchost.exe") return [];
    return services.filter((service) => service.pid === process.pid);
  }

  $: hostedServices = servicesFor(hovered, $servicesCacheStore.services);

  $: shownServices = hostedServices.slice(0, 3);
  $: extraServices = hostedServices.length - shownServices.length;

  $: vw = typeof window !== "undefined" ? window.innerWidth : 1280;
  $: vh = typeof window !== "undefined" ? window.innerHeight : 800;
  // Flip to the left/up near the edges, then clamp inside the viewport
  $: flipX = x + 12 + CARD_W > vw;
  $: flipY = y + 12 + CARD_H > vh;
  $: left = flipX
    ? Math.max(8, x - CARD_W - 12)
    : Math.min(vw - CARD_W - 8, x + 12);
  $: top = flipY
    ? Math.max(8, y - CARD_H - 12)
    : Math.min(vh - CARD_H - 8, y + 12);
</script>

{#if visible && hovered}
  <div class="hover-card" style="left: {left}px; top: {top}px">
    <div class="hover-title">
      <span class="hover-name">{hovered.name}</span>
      <span class="hover-pid">{$t("details.pid")} {hovered.pid}</span>
    </div>

    {#if metadata && (metadata.company || metadata.description)}
      <div class="hover-company">
        {metadata.company}
        {#if metadata.company && metadata.description}
          <span class="hover-dot">·</span>
        {/if}
        {metadata.description}
      </div>
    {/if}

    {#if metadata && metadata.version}
      <div class="hover-version">
        {$t("details.tooltipVersion", { version: metadata.version })}
      </div>
    {/if}

    {#if metadata && metadata.elevated}
      <div class="hover-elevated">
        <Fa icon={faUserShield} />
        <span>{$t("details.tooltipElevated")}</span>
      </div>
    {/if}

    {#if hostedServices.length > 0}
      <div class="hover-services">
        <div class="hover-services-label">
          {$t("details.tooltipServices", { count: hostedServices.length })}
        </div>
        {#each shownServices as service (service.name)}
          <div class="hover-service">
            {service.display_name || service.name}
          </div>
        {/each}
        {#if extraServices > 0}
          <div class="hover-service hover-more">
            {$t("details.tooltipMore", { count: extraServices })}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .hover-card {
    position: fixed;
    z-index: 900; /* below the modal backdrop so a modal always covers it */
    max-width: 320px;
    padding: 10px 12px;
    background: var(--mantle);
    border: 1px solid var(--surface1);
    border-radius: 8px;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.35);
    font-size: 12px;
    color: var(--text);
    pointer-events: none;
  }

  .hover-title {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .hover-name {
    font-weight: 600;
    font-size: 13px;
  }

  .hover-pid {
    color: var(--subtext0);
    white-space: nowrap;
  }

  .hover-company {
    margin-top: 6px;
    color: var(--subtext1);
    word-break: break-word;
  }

  .hover-dot {
    color: var(--overlay0);
  }

  .hover-version {
    margin-top: 4px;
    color: var(--subtext0);
  }

  .hover-elevated {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    padding: 1px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--red) 18%, transparent);
    color: var(--red);
  }

  .hover-elevated :global(svg) {
    width: 10px;
    height: 10px;
  }

  .hover-services {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--surface0);
  }

  .hover-services-label {
    color: var(--subtext0);
    font-weight: 500;
  }

  .hover-service {
    margin-top: 2px;
    color: var(--subtext1);
    word-break: break-word;
  }

  .hover-more {
    color: var(--overlay0);
  }
</style>
