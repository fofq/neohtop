<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { platform } from "@tauri-apps/plugin-os";
  import Fa from "svelte-fa";
  import {
    faCubes,
    faMicrochip,
    faRefresh,
  } from "@fortawesome/free-solid-svg-icons";
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/utils";
  import { SearchInput } from "$lib/components";
  import type { DriverInfo, ModuleInfo, Process } from "$lib/types";

  export let process: Process;

  const isWindows = platform() === "windows";

  /** 50 rows per page keeps the huge driver list cheap to render. */
  const PAGE_SIZE = 50;

  type SubTab = "modules" | "drivers";
  let subTab: SubTab = "modules";

  let loadedPid: number | null = null;
  let modules: ModuleInfo[] | null = null;
  let modulesError: string | null = null;
  let loadingModules = false;

  let drivers: DriverInfo[] | null = null;
  let driversError: string | null = null;
  let loadingDrivers = false;

  let filter = "";
  let page = 1;

  // Reload when the modal switches to another process; staged state (the
  // filter) resets with it, mirroring the priority-info loading pattern.
  $: processPid = process ? process.pid : null;
  $: if (isWindows && processPid !== null && processPid !== loadedPid) {
    loadedPid = processPid;
    modules = null;
    modulesError = null;
    drivers = null;
    driversError = null;
    filter = "";
    page = 1;
    loadModules(processPid);
  }

  async function loadModules(pid: number) {
    if (loadingModules) return;
    loadingModules = true;
    modulesError = null;
    try {
      modules = await invoke<ModuleInfo[]>("list_process_modules", { pid });
    } catch (e) {
      modulesError = e instanceof Error ? e.message : String(e);
    } finally {
      loadingModules = false;
    }
  }

  async function loadDrivers() {
    if (loadingDrivers) return;
    loadingDrivers = true;
    driversError = null;
    try {
      drivers = await invoke<DriverInfo[]>("list_drivers");
    } catch (e) {
      driversError = e instanceof Error ? e.message : String(e);
    } finally {
      loadingDrivers = false;
    }
  }

  function switchSubTab(tab: SubTab) {
    subTab = tab;
    filter = "";
    page = 1;
    // Drivers are read-only and big; load them once, on first entry
    if (tab === "drivers" && !drivers && !loadingDrivers && process) {
      loadDrivers();
    }
  }

  function refreshCurrent() {
    if (!process) return;
    page = 1;
    if (subTab === "modules") {
      loadModules(process.pid);
    } else {
      loadDrivers();
    }
  }

  function baseAddressHex(base: number): string {
    return `0x${base.toString(16).toUpperCase()}`;
  }

  function applyFilter<T extends { name: string }>(
    list: T[] | null,
    term: string,
  ): T[] {
    if (!list) return [];
    const query = term.trim().toLowerCase();
    if (!query) return list;
    return list.filter((item) => item.name.toLowerCase().includes(query));
  }

  $: shownList = subTab === "modules" ? modules : drivers;
  $: filteredList = applyFilter(shownList, filter);
  $: totalPages = Math.max(1, Math.ceil(filteredList.length / PAGE_SIZE));
  $: if (page > totalPages) page = totalPages;
  $: pageItems = filteredList.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
</script>

{#if !isWindows}
  <div class="modules-unavailable">
    <Fa icon={faCubes} />
    <p>{$t("modules.unavailable")}</p>
  </div>
{:else}
  <div class="modules-content">
    <div class="modules-toolbar">
      <div class="view-toggle">
        <button
          class:active={subTab === "modules"}
          on:click={() => switchSubTab("modules")}
        >
          <Fa icon={faCubes} />
          <span>{$t("modules.modulesTab")}</span>
        </button>
        <button
          class:active={subTab === "drivers"}
          on:click={() => switchSubTab("drivers")}
        >
          <Fa icon={faMicrochip} />
          <span>{$t("modules.driversTab")}</span>
        </button>
      </div>
      <div class="modules-search">
        <SearchInput
          bind:value={filter}
          placeholder={$t("modules.searchPlaceholder")}
          on:input={() => (page = 1)}
        />
      </div>
      <button
        class="modules-refresh"
        on:click={refreshCurrent}
        disabled={loadingModules || loadingDrivers}
        aria-label={$t("modules.refresh")}
      >
        <span class="refresh-icon">
          {#if loadingModules || loadingDrivers}
            <span class="spinner"></span>
          {:else}
            <Fa icon={faRefresh} />
          {/if}
        </span>
      </button>
    </div>

    {#if subTab === "modules" && modulesError}
      <div class="control-note error">
        <div>{modulesError}</div>
        <div class="control-hint">{$t("details.adminHint")}</div>
      </div>
    {:else if subTab === "drivers" && driversError}
      <div class="control-note error">
        <div>{driversError}</div>
        <div class="control-hint">{$t("details.adminHint")}</div>
      </div>
    {/if}

    {#if (subTab === "modules" && loadingModules && !modules) || (subTab === "drivers" && loadingDrivers && !drivers)}
      <div class="modules-status">
        <div class="spinner"></div>
      </div>
    {:else if filteredList.length === 0}
      <div class="modules-status">
        {filter
          ? $t("modules.noResults")
          : subTab === "modules"
            ? $t("modules.empty")
            : $t("modules.emptyDrivers")}
      </div>
    {:else}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>{$t("modules.name")}</th>
              <th class="size-col">{$t("modules.size")}</th>
              <th class="base-col">{$t("modules.baseAddress")}</th>
            </tr>
          </thead>
          <tbody>
            {#each pageItems as item (item.base_address + item.name)}
              <tr>
                <td>
                  <div class="module-name">{item.name}</div>
                  {#if item.path}
                    <div class="module-path">{item.path}</div>
                  {/if}
                </td>
                {#if subTab === "modules"}
                  <td class="mono size-col">
                    {formatBytes((item as ModuleInfo).size)}
                  </td>
                {/if}
                <td class="mono base-col"
                  >{baseAddressHex(item.base_address)}</td
                >
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="modules-footer">
        <span class="modules-count">
          {$t("modules.count", {
            shown: pageItems.length,
            total: filteredList.length,
          })}
        </span>
        <div class="pager">
          <button
            class="pager-btn"
            on:click={() => (page -= 1)}
            disabled={page <= 1}
            aria-label={$t("modules.previousPage")}
          >
            ‹
          </button>
          <span class="pager-label">
            {$t("pagination.pageOf", { current: page, total: totalPages })}
          </span>
          <button
            class="pager-btn"
            on:click={() => (page += 1)}
            disabled={page >= totalPages}
            aria-label={$t("modules.nextPage")}
          >
            ›
          </button>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .modules-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .modules-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .view-toggle {
    display: inline-flex;
    height: 28px;
    overflow: hidden;
    border: 1px solid var(--surface1);
    border-radius: 6px;
    flex-shrink: 0;
  }

  .view-toggle button {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    padding: 0 10px;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: none;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
  }

  .view-toggle button + button {
    border-left: 1px solid var(--surface1);
  }

  .view-toggle button.active {
    color: var(--base);
    background: var(--blue);
  }

  .view-toggle button :global(svg) {
    width: 10px;
    height: 10px;
  }

  /* Layout only — the shared SearchInput owns the input's look */
  .modules-search {
    flex: 1;
    min-width: 0;
  }

  .modules-refresh {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    flex-shrink: 0;
  }

  .modules-refresh:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface1);
  }

  .modules-refresh:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .modules-refresh :global(svg) {
    width: 11px;
    height: 11px;
  }

  .modules-status {
    padding: 16px;
    border-radius: 6px;
    background: var(--mantle);
    color: var(--subtext0);
    font-size: 13px;
    display: flex;
    justify-content: center;
  }

  .table-wrap {
    overflow-x: auto;
    max-height: 420px;
    overflow-y: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  th {
    text-align: left;
    padding: 8px;
    color: var(--subtext0);
    font-weight: 500;
    border-bottom: 1px solid var(--surface1);
    position: sticky;
    top: 0;
    background: var(--base);
  }

  td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--surface0);
    color: var(--text);
    vertical-align: top;
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
    font-size: 12px;
    white-space: nowrap;
  }

  .module-name {
    color: var(--text);
  }

  .module-path {
    margin-top: 2px;
    font-size: 11px;
    color: var(--subtext0);
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
    word-break: break-all;
  }

  .size-col {
    width: 90px;
    white-space: nowrap;
  }

  .base-col {
    width: 130px;
    white-space: nowrap;
  }

  .modules-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .modules-count {
    font-size: 12px;
    color: var(--subtext0);
  }

  .pager {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .pager-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .pager-btn:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface1);
  }

  .pager-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .pager-label {
    font-size: 12px;
    color: var(--subtext0);
  }

  .modules-unavailable {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 32px 16px;
    color: var(--subtext0);
    background: var(--mantle);
    border-radius: 8px;
  }

  .modules-unavailable :global(svg) {
    width: 20px;
    height: 20px;
  }

  .modules-unavailable p {
    margin: 0;
    font-size: 13px;
  }

  .control-note {
    padding: 10px 12px;
    border-radius: 6px;
    background: var(--mantle);
    font-size: 12px;
    color: var(--text);
    word-break: break-word;
  }

  .control-note.error {
    color: var(--red);
    border: 1px solid color-mix(in srgb, var(--red) 40%, transparent);
  }

  .control-note .control-hint {
    margin-top: 6px;
  }

  .control-hint {
    color: var(--subtext0);
    font-size: 12px;
  }

  .refresh-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--surface2);
    border-top-color: var(--text);
    border-radius: 50%;
    animation: modules-spin 0.8s linear infinite;
  }

  @keyframes modules-spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
