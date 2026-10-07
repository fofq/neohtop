<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { platform } from "@tauri-apps/plugin-os";
  import Fa from "svelte-fa";
  import {
    faFileCircleQuestion,
    faMagnifyingGlass,
    faCircleInfo,
    faXmark,
  } from "@fortawesome/free-solid-svg-icons";
  import { backToTop } from "$lib/actions/backToTop";
  import { KillProcessModal, Modal, SearchInput } from "$lib/components";
  import { t } from "$lib/i18n";
  import { processStore } from "$lib/stores/index";
  import { withElevationHint } from "$lib/utils";
  import type { FileLocker, Process } from "$lib/types";

  export let show = false;
  export let onClose: () => void;

  const isWindows = platform() === "windows";

  let path = "";
  // Path the current result set belongs to; shown above the results
  let lastQuery = "";
  let lockers: FileLocker[] = [];
  let isLoading = false;
  let error: string | null = null;

  // Kill flow reuses the shared KillProcessModal confirm dialog
  let killTarget: { pid: number; name: string } | null = null;
  let isKilling = false;
  let killError: string | null = null;

  // Resolve PIDs against the current process snapshot so rows can jump to
  // the regular process details modal
  $: processByPid = new Map($processStore.processes.map((p) => [p.pid, p]));

  async function findLockers() {
    const query = path.trim();
    if (!query || isLoading) return;
    isLoading = true;
    error = null;
    killError = null;
    try {
      lockers = await invoke<FileLocker[]>("get_file_lockers", { path: query });
      lastQuery = query;
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
      lockers = [];
      lastQuery = "";
    } finally {
      isLoading = false;
    }
  }

  // The details modal stacks below this one (both z-index 1000, DOM order
  // decides), so the panel closes first and hands over to the table row
  function showDetails(pid: number) {
    const process = processByPid.get(pid);
    if (process) {
      onClose();
      processStore.showProcessDetails(process);
    }
  }

  function askKill(locker: FileLocker) {
    killError = null;
    killTarget = {
      pid: locker.pid,
      name: locker.app_name || locker.short_name,
    };
  }

  async function handleKillConfirm() {
    if (!killTarget || isKilling) return;
    isKilling = true;
    try {
      const success = await invoke<boolean>("kill_process", {
        pid: killTarget.pid,
      });
      if (!success) {
        throw new Error("Failed to kill process");
      }
      killTarget = null;
      // The file is (partly) released now; re-run the query on the same path
      await findLockers();
    } catch (e) {
      killError = withElevationHint(e instanceof Error ? e.message : String(e));
      killTarget = null;
    } finally {
      isKilling = false;
    }
  }
</script>

<Modal {show} title={$t("lockers.title")} maxWidth="720px" {onClose}>
  {#if !isWindows}
    <div class="lockers-unavailable">
      <Fa icon={faFileCircleQuestion} />
      <p>{$t("lockers.unavailable")}</p>
    </div>
  {:else}
    <div class="lockers-content">
      <div class="lockers-toolbar">
        <div class="lockers-path">
          <SearchInput
            bind:value={path}
            placeholder={$t("lockers.pathPlaceholder")}
            on:keydown={(e) => e.key === "Enter" && findLockers()}
          />
        </div>
        <button
          class="lockers-search"
          on:click={findLockers}
          disabled={isLoading || !path.trim()}
          aria-label={$t("lockers.searchAria")}
        >
          {#if isLoading}
            <span class="spinner"></span>
          {:else}
            <Fa icon={faMagnifyingGlass} />
          {/if}
          <span>{$t("lockers.search")}</span>
        </button>
      </div>

      {#if killError}
        <div class="lockers-error">{killError}</div>
      {/if}

      {#if error}
        <div class="lockers-error">{error}</div>
      {:else if isLoading && !lastQuery}
        <div class="lockers-status">
          <div class="spinner"></div>
        </div>
      {:else if !lastQuery}
        <div class="lockers-status">
          <Fa icon={faCircleInfo} />
          <span>{$t("lockers.hint")}</span>
        </div>
      {:else}
        <div class="count-row">
          {$t("lockers.resultCount", {
            count: lockers.length,
            path: lastQuery,
          })}
        </div>
        {#if lockers.length === 0}
          <div class="lockers-status">{$t("lockers.empty")}</div>
        {:else}
          <div class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>{$t("lockers.process")}</th>
                  <th>{$t("lockers.pid")}</th>
                  <th>{$t("lockers.service")}</th>
                  <th class="actions-col"></th>
                </tr>
              </thead>
              <tbody>
                {#each lockers as locker}
                  <tr>
                    <td class="process-name"
                      >{locker.app_name || locker.short_name || "-"}</td
                    >
                    <td class="mono">{locker.pid}</td>
                    <td class="mono">{locker.short_name || "-"}</td>
                    <td class="actions-col">
                      <button
                        class="row-action"
                        disabled={!processByPid.has(locker.pid)}
                        on:click={() => showDetails(locker.pid)}
                        title={processByPid.has(locker.pid)
                          ? $t("lockers.showDetails")
                          : $t("lockers.notInSnapshot")}
                        aria-label={$t("lockers.showDetails")}
                      >
                        <Fa icon={faCircleInfo} />
                      </button>
                      <button
                        class="row-action danger"
                        on:click={() => askKill(locker)}
                        title={$t("lockers.kill")}
                        aria-label={$t("lockers.kill")}
                      >
                        <Fa icon={faXmark} />
                      </button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</Modal>

<KillProcessModal
  show={killTarget !== null}
  process={killTarget}
  {isKilling}
  onClose={() => (killTarget = null)}
  onConfirm={handleKillConfirm}
/>

<style>
  .lockers-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .lockers-toolbar {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  /* Layout only — the shared SearchInput owns the input's look */
  .lockers-path {
    flex: 1;
    min-width: 0;
  }

  .lockers-search {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    justify-content: center;
    min-width: 92px;
    height: 28px;
    padding: 0 12px;
    font-size: 12px;
    color: var(--text);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
  }

  .lockers-search:hover:not(:disabled) {
    background: var(--surface1);
  }

  .lockers-search:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .lockers-search :global(svg) {
    font-size: 11px;
  }

  .count-row {
    font-size: 12px;
    color: var(--subtext0);
    word-break: break-all;
  }

  .table-wrap {
    max-height: 50vh;
    overflow: auto;
    border: 1px solid var(--surface0);
    border-radius: 6px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  thead th {
    position: sticky;
    top: 0;
    padding: 8px 10px;
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
    color: var(--subtext0);
    background: var(--mantle);
    border-bottom: 1px solid var(--surface0);
  }

  .actions-col {
    width: 64px;
    text-align: center;
  }

  tbody td {
    padding: 6px 10px;
    white-space: nowrap;
    color: var(--text);
    border-bottom: 1px solid var(--surface0);
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  tbody tr:hover {
    background: var(--mantle);
  }

  .process-name {
    font-weight: 600;
    color: var(--blue);
  }

  .mono {
    font-family: monospace;
    font-size: 12px;
  }

  .row-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    margin-left: 4px;
    padding: 0;
    color: var(--subtext0);
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    opacity: 0;
    transition: all 0.15s ease;
  }

  tbody tr:hover .row-action,
  .row-action:focus-visible {
    opacity: 1;
  }

  .row-action:hover:not(:disabled) {
    color: var(--blue);
    background: var(--surface1);
  }

  .row-action.danger:hover:not(:disabled) {
    color: var(--red);
  }

  .row-action:disabled {
    cursor: not-allowed;
    opacity: 0.3;
  }

  .row-action :global(svg) {
    font-size: 11px;
  }

  .lockers-status {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: center;
    padding: 32px;
    font-size: 13px;
    color: var(--subtext0);
  }

  .lockers-unavailable {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: center;
    padding: 40px 16px;
    color: var(--subtext0);
  }

  .lockers-unavailable :global(svg) {
    font-size: 28px;
    color: var(--overlay0);
  }

  .lockers-unavailable p {
    margin: 0;
    font-size: 13px;
  }

  .lockers-error {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--red);
    background: var(--surface0);
    border: 1px solid var(--red);
    border-radius: 6px;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--surface1);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
