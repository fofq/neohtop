<script lang="ts">
  import { Modal } from "$lib/components";
  import { formatBytes } from "$lib/utils";
  import { t, statusLabel } from "$lib/i18n";
  import type { Process } from "$lib/types";
  import { invoke } from "@tauri-apps/api/core";
  import { platform } from "@tauri-apps/plugin-os";
  import { processStore } from "$lib/stores/index";
  import Fa from "svelte-fa";
  import {
    faMemory,
    faMicrochip,
    faCodeFork,
    faTerminal,
    faList,
    faSliders,
    faCircleInfo,
    faChartLine,
    faGears,
    faCubes,
  } from "@fortawesome/free-solid-svg-icons";
  import {
    ProcessPerformanceTab,
    ProcessServicesTab,
    ProcessModulesTab,
  } from "$lib/components";

  export let show = false;
  export let process: Process | null = null;
  export let onClose: () => void;
  export let processes: Process[] = [];
  export let onShowDetails: (process: Process) => void;

  // --- Tabs ---
  type DetailsTab = "general" | "performance" | "services" | "modules";
  let activeTab: DetailsTab = "general";
  // Reopening the modal always lands back on the general tab
  $: if (!show) {
    activeTab = "general";
  }

  // Performance ring buffer of the selected process, fed once per polling
  // cycle by the processes store; empty until samples were collected
  $: performanceHistory = $processStore.selectedHistory.points;

  $: childProcesses = process
    ? processes.filter((p) => p.ppid === process.pid)
    : [];

  // --- Process control (priority / efficiency / affinity) ---
  // Mirrors the backend ProcessPriorityInfo (Windows only; other platforms
  // return an error string from get_process_priority_info, so the section
  // is simply hidden there).
  interface ProcessPriorityInfo {
    priority_class: string;
    // Affinity masks arrive as decimal strings: 64-bit values lose
    // precision as JSON numbers above 2^53
    affinity_mask: string;
    system_affinity_mask: string;
    efficiency_mode: boolean;
  }

  // Class names accepted by the backend set_process_priority command.
  const PRIORITY_CLASSES = [
    "idle",
    "below_normal",
    "normal",
    "above_normal",
    "high",
    "realtime",
  ];

  let loadedPid: number | null = null;
  let priorityInfo: ProcessPriorityInfo | null = null;
  let priorityError: string | null = null;
  let loadingPriority = false;

  // Staged edits; applied only when the user clicks the apply button.
  let selectedPriority = "normal";
  let efficiencyOn = false;
  let selectedCores: Set<number> = new Set();

  let applying = false;
  let controlError: string | null = null;
  let affinityConfirmPending = false;

  $: isWindows = platform() === "windows";
  $: priorityPid = show && process && isWindows ? process.pid : null;
  // (Re)load when the modal opens for a process, once per PID: the details
  // process object is replaced on every refresh snapshot, but staged edits
  // must survive that, so don't key off object identity.
  $: if (priorityPid === null) {
    if (loadedPid !== null) {
      loadedPid = null;
      resetControlState();
    }
  } else if (priorityPid !== loadedPid) {
    loadPriorityInfo(priorityPid);
  }

  $: systemCores = priorityInfo
    ? coresFromMask(priorityInfo.system_affinity_mask)
    : new Set<number>();

  $: priorityDirty =
    priorityInfo !== null && selectedPriority !== priorityInfo.priority_class;
  $: efficiencyDirty =
    priorityInfo !== null && efficiencyOn !== priorityInfo.efficiency_mode;
  $: affinityDirty =
    priorityInfo !== null &&
    maskFromCores(selectedCores) !== priorityInfo.affinity_mask;
  $: hasControlChanges = priorityDirty || efficiencyDirty || affinityDirty;

  function resetControlState() {
    priorityInfo = null;
    priorityError = null;
    loadingPriority = false;
    selectedPriority = "normal";
    efficiencyOn = false;
    selectedCores = new Set();
    applying = false;
    controlError = null;
    affinityConfirmPending = false;
  }

  async function loadPriorityInfo(pid: number) {
    loadingPriority = true;
    loadedPid = pid;
    priorityError = null;
    controlError = null;
    affinityConfirmPending = false;
    try {
      const info = await invoke<ProcessPriorityInfo>(
        "get_process_priority_info",
        { pid },
      );
      priorityInfo = info;
      selectedPriority = info.priority_class;
      efficiencyOn = info.efficiency_mode;
      selectedCores = coresFromMask(info.affinity_mask);
    } catch (e: unknown) {
      priorityInfo = null;
      priorityError = e instanceof Error ? e.message : String(e);
    } finally {
      loadingPriority = false;
    }
  }

  // Masks travel as decimal strings and are handled with BigInt because
  // 64-bit values lose precision as JS numbers above 2^53.
  function coresFromMask(mask: string): Set<number> {
    const value = BigInt(mask);
    const cores = new Set<number>();
    for (let bit = 0; bit < 64; bit++) {
      if (((value >> BigInt(bit)) & 1n) !== 0n) cores.add(bit);
    }
    return cores;
  }

  function maskFromCores(cores: Set<number>): string {
    let mask = 0n;
    for (const bit of cores) mask += 2n ** BigInt(bit);
    return mask.toString();
  }

  function toggleCore(bit: number) {
    const next = new Set(selectedCores);
    if (next.has(bit)) {
      next.delete(bit);
    } else {
      next.add(bit);
    }
    selectedCores = next;
    affinityConfirmPending = false;
  }

  function resetControlChanges() {
    if (!priorityInfo) return;
    selectedPriority = priorityInfo.priority_class;
    efficiencyOn = priorityInfo.efficiency_mode;
    selectedCores = coresFromMask(priorityInfo.affinity_mask);
    affinityConfirmPending = false;
    controlError = null;
  }

  async function applyControlChanges() {
    if (!process || !priorityInfo || applying || !hasControlChanges) return;
    // Changing CPU affinity can starve or destabilize a process, so it
    // needs a second explicit confirmation click before anything is sent.
    if (affinityDirty && !affinityConfirmPending) {
      affinityConfirmPending = true;
      return;
    }
    applying = true;
    controlError = null;
    const pid = process.pid;
    try {
      if (priorityDirty && selectedPriority) {
        const ok = await invoke<boolean>("set_process_priority", {
          pid,
          class: selectedPriority,
        });
        if (!ok) throw new Error("Failed to set the priority class");
      }
      if (affinityDirty) {
        const ok = await invoke<boolean>("set_process_affinity", {
          pid,
          mask: maskFromCores(selectedCores),
        });
        if (!ok) throw new Error("Failed to set the CPU affinity mask");
      }
      if (efficiencyDirty) {
        const ok = await invoke<boolean>("set_process_efficiency", {
          pid,
          enabled: efficiencyOn,
        });
        if (!ok) throw new Error("Failed to set the efficiency mode");
      }
      // Re-read the authoritative state instead of assuming every call
      // landed; also clears the staged dirty flags.
      await loadPriorityInfo(pid);
    } catch (e: unknown) {
      controlError = e instanceof Error ? e.message : String(e);
    } finally {
      applying = false;
    }
  }
</script>

<Modal
  {show}
  title={$t("details.title", {
    name: process ? process.name.slice(0, 10) : $t("details.unknownProcess"),
  })}
  maxWidth="1000px"
  {onClose}
>
  {#if process}
    <div class="modal-content">
      <!-- Header Stats -->
      <div class="header-stats">
        <div class="stat-item">
          <div class="stat-label">{$t("details.pid")}</div>
          <div class="stat-value">{process.pid}</div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{$t("details.status")}</div>
          <div
            class="stat-value status"
            class:running={process.status === "Running"}
          >
            {$statusLabel(process.status)}
          </div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{$t("details.cpu")}</div>
          <div class="stat-value">{process.cpu_usage.toFixed(1)}%</div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{$t("details.memory")}</div>
          <div class="stat-value">{formatBytes(process.memory_usage)}</div>
        </div>
      </div>

      <!-- Tabs -->
      <div class="tab-bar" role="tablist">
        <button
          class="tab-btn"
          class:active={activeTab === "general"}
          role="tab"
          aria-selected={activeTab === "general"}
          on:click={() => (activeTab = "general")}
        >
          <Fa icon={faCircleInfo} />
          <span>{$t("details.tabGeneral")}</span>
        </button>
        <button
          class="tab-btn"
          class:active={activeTab === "performance"}
          role="tab"
          aria-selected={activeTab === "performance"}
          on:click={() => (activeTab = "performance")}
        >
          <Fa icon={faChartLine} />
          <span>{$t("details.tabPerformance")}</span>
        </button>
        <button
          class="tab-btn"
          class:active={activeTab === "services"}
          role="tab"
          aria-selected={activeTab === "services"}
          on:click={() => (activeTab = "services")}
        >
          <Fa icon={faGears} />
          <span>{$t("details.tabServices")}</span>
        </button>
        <button
          class="tab-btn"
          class:active={activeTab === "modules"}
          role="tab"
          aria-selected={activeTab === "modules"}
          on:click={() => (activeTab = "modules")}
        >
          <Fa icon={faCubes} />
          <span>{$t("details.tabModules")}</span>
        </button>
      </div>

      <!-- Main Content -->
      {#if activeTab === "general"}
        <div class="content-grid">
          <!-- Left Column -->
          <div class="content-column">
            <!-- Process Info -->
            <div class="card">
              <div class="card-header">
                <Fa icon={faMicrochip} />
                <span>{$t("details.processInfo")}</span>
              </div>
              <div class="card-content">
                <div class="info-grid">
                  <div class="info-item">
                    <span class="info-label">{$t("details.name")}</span>
                    <span class="info-value">{process.name}</span>
                  </div>
                  <div class="info-item">
                    <span class="info-label">{$t("details.user")}</span>
                    <span class="info-value">{process.user}</span>
                  </div>
                  <div class="info-item">
                    <span class="info-label">{$t("details.parentPid")}</span>
                    <!-- svelte-ignore a11y_click_events_have_key_events -->
                    <!-- svelte-ignore a11y_no_static_element_interactions -->
                    <span
                      class="info-value clickable"
                      on:click={() => {
                        const parent = processes.find(
                          (p) => p.pid === process.ppid,
                        );
                        if (parent) onShowDetails(parent);
                      }}
                    >
                      {process.ppid}
                    </span>
                  </div>
                  <div class="info-item">
                    <span class="info-label">{$t("details.sessionId")}</span>
                    <span class="info-value">{process.session_id}</span>
                  </div>
                </div>
              </div>
            </div>

            <!-- Resource Usage -->
            <div class="card">
              <div class="card-header">
                <Fa icon={faMemory} />
                <span>{$t("details.resourceUsage")}</span>
              </div>
              <div class="card-content">
                <div class="resource-grid">
                  <div class="resource-item">
                    <div class="resource-header">
                      <span>{$t("details.cpuUsage")}</span>
                      <span class="resource-value"
                        >{process.cpu_usage.toFixed(1)}%</span
                      >
                    </div>
                    <div class="progress-bar">
                      <div
                        class="progress-fill"
                        style="width: {process.cpu_usage}%"
                        class:high={process.cpu_usage > 50}
                        class:critical={process.cpu_usage > 80}
                      ></div>
                    </div>
                  </div>
                  <div class="resource-item">
                    <div class="resource-header">
                      <span>{$t("details.memoryUsage")}</span>
                    </div>
                    <div class="memory-stats">
                      <div>
                        {$t("details.physical", {
                          size: formatBytes(process.memory_usage),
                        })}
                      </div>
                      <div>
                        {$t("details.virtual", {
                          size: formatBytes(process.virtual_memory),
                        })}
                      </div>
                    </div>
                  </div>
                  <div class="resource-item">
                    <div class="resource-header">
                      <span>{$t("details.diskIO")}</span>
                    </div>
                    <div class="disk-stats">
                      <div>
                        {$t("details.read", {
                          size: formatBytes(process.disk_usage[0]),
                        })}
                      </div>
                      <div>
                        {$t("details.written", {
                          size: formatBytes(process.disk_usage[1]),
                        })}
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Right Column -->
          <div class="content-column">
            <!-- Command -->
            <div class="card">
              <div class="card-header">
                <Fa icon={faTerminal} />
                <span>{$t("details.command")}</span>
              </div>
              <div class="card-content">
                <div class="command-text">{process.command}</div>
                <div class="path-text">{process.root}</div>
              </div>
            </div>

            <!-- Child Processes -->
            {#if childProcesses.length > 0}
              <div class="card">
                <div class="card-header">
                  <Fa icon={faCodeFork} />
                  <span
                    >{$t("details.childProcesses", {
                      count: childProcesses.length,
                    })}</span
                  >
                </div>
                <div class="card-content">
                  <table class="process-table">
                    <thead>
                      <tr>
                        <th>{$t("details.name")}</th>
                        <th>{$t("details.pid")}</th>
                        <th>{$t("details.cpu")}</th>
                        <th>{$t("details.memory")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each childProcesses as child}
                        <tr
                          class="clickable"
                          on:click={() => onShowDetails(child)}
                        >
                          <td>{child.name}</td>
                          <td>{child.pid}</td>
                          <td>{child.cpu_usage.toFixed(1)}%</td>
                          <td>{formatBytes(child.memory_usage)}</td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              </div>
            {/if}

            <!-- Environment Variables -->
            {#if process.environ.length > 0}
              <div class="card">
                <div class="card-header">
                  <Fa icon={faList} />
                  <span>{$t("details.envVars")}</span>
                </div>
                <div class="card-content">
                  <div class="env-list">
                    {#each process.environ as env}
                      <div class="env-item">{env}</div>
                    {/each}
                  </div>
                </div>
              </div>
            {/if}

            <!-- Process Control -->
            {#if isWindows}
              <div class="card">
                <div class="card-header">
                  <Fa icon={faSliders} />
                  <span>{$t("details.controlSection")}</span>
                </div>
                <div class="card-content">
                  {#if loadingPriority && !priorityInfo}
                    <div class="control-note">
                      {$t("details.controlLoading")}
                    </div>
                  {:else if priorityError}
                    <div class="control-note error">
                      <div>{priorityError}</div>
                      <div class="control-hint">
                        {$t("details.adminHint")}
                      </div>
                    </div>
                  {:else if priorityInfo}
                    <!-- Priority class -->
                    <div class="control-group">
                      <div class="control-label">
                        {$t("details.currentPriority", {
                          class: priorityInfo.priority_class,
                        })}
                      </div>
                      <div class="priority-options">
                        {#each PRIORITY_CLASSES as cls (cls)}
                          <label class="priority-option">
                            <input
                              type="radio"
                              name="priority-class"
                              value={cls}
                              bind:group={selectedPriority}
                              disabled={applying}
                            />
                            <span>{$t(`details.priority.${cls}`)}</span>
                          </label>
                        {/each}
                      </div>
                    </div>

                    <!-- Efficiency mode -->
                    <div class="control-group">
                      <label class="efficiency-toggle">
                        <input
                          type="checkbox"
                          bind:checked={efficiencyOn}
                          disabled={applying}
                        />
                        <span>{$t("details.efficiencyMode")}</span>
                      </label>
                      <div class="control-hint">
                        {$t("details.efficiencyDesc")}
                      </div>
                    </div>

                    <!-- CPU affinity -->
                    <div class="control-group">
                      <div class="control-label">
                        {$t("details.affinity", { count: systemCores.size })}
                      </div>
                      <div class="core-grid">
                        {#each [...systemCores] as bit (bit)}
                          <button
                            type="button"
                            class="core-chip"
                            class:active={selectedCores.has(bit)}
                            disabled={applying}
                            aria-pressed={selectedCores.has(bit)}
                            on:click={() => toggleCore(bit)}
                          >
                            CPU {bit}
                          </button>
                        {/each}
                      </div>
                      {#if selectedCores.size === 0}
                        <div class="control-hint warning">
                          {$t("details.noCoreSelected")}
                        </div>
                      {/if}
                    </div>

                    <!-- Apply / reset -->
                    <div class="control-actions">
                      <button
                        class="btn-control secondary"
                        on:click={resetControlChanges}
                        disabled={applying || !hasControlChanges}
                      >
                        {$t("details.resetChanges")}
                      </button>
                      <button
                        class="btn-control primary"
                        class:confirm={affinityConfirmPending}
                        on:click={applyControlChanges}
                        disabled={applying ||
                          !hasControlChanges ||
                          selectedCores.size === 0}
                      >
                        {#if applying}
                          <div class="spinner"></div>
                          <span>{$t("details.applying")}</span>
                        {:else if affinityConfirmPending}
                          {$t("details.confirmApply")}
                        {:else}
                          {$t("details.applyChanges")}
                        {/if}
                      </button>
                    </div>
                    {#if affinityConfirmPending}
                      <div class="control-hint warning">
                        {$t("details.affinityConfirmHint")}
                      </div>
                    {/if}
                    {#if controlError}
                      <div class="control-note error">
                        <div>{controlError}</div>
                        <div class="control-hint">
                          {$t("details.adminHint")}
                        </div>
                      </div>
                    {/if}
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        </div>
      {:else if activeTab === "performance"}
        <ProcessPerformanceTab {process} history={performanceHistory} />
      {:else if activeTab === "services"}
        <ProcessServicesTab {process} />
      {:else if activeTab === "modules"}
        <ProcessModulesTab {process} />
      {/if}
    </div>
  {/if}
</Modal>

<style>
  /* Base Modal Content */
  .modal-content {
    display: flex;
    flex-direction: column;
    gap: 24px;
    font-size: 13px;
    color: var(--text);
  }

  /* Header Stats */
  .header-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: 16px;
    padding: 16px;
    background: var(--surface0);
    border-radius: 8px;
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .stat-label {
    font-size: 12px;
    color: var(--subtext0);
    font-weight: 500;
  }

  .stat-value {
    font-size: 16px;
    font-weight: 600;
  }

  .stat-value.status {
    color: var(--subtext0);
  }

  .stat-value.status.running {
    color: var(--green);
  }

  /* Tabs */
  .tab-bar {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--surface1);
  }

  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    margin-bottom: -1px;
    font-size: 13px;
    color: var(--subtext0);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .tab-btn:hover {
    color: var(--text);
  }

  .tab-btn.active {
    color: var(--text);
    border-bottom-color: var(--blue);
  }

  .tab-btn :global(svg) {
    width: 12px;
    height: 12px;
  }

  /* Main Content Grid */
  .content-grid {
    display: grid;
    grid-template-columns: minmax(300px, 0.4fr) minmax(400px, 0.6fr);
    gap: 24px;
  }

  .content-column {
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-width: 0; /* Prevent overflow issues */
  }

  /* Cards */
  .card {
    background: var(--surface0);
    border-radius: 8px;
    overflow: hidden;
    min-width: 0; /* Prevent overflow issues */
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    background: var(--surface1);
    color: var(--subtext0);
    font-weight: 500;
  }

  .card-header :global(svg) {
    width: 14px;
    height: 14px;
    color: var(--blue);
  }

  .card-content {
    padding: 16px;
    overflow: auto;
  }

  /* Info Grid */
  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 16px;
  }

  .info-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .info-label {
    color: var(--subtext0);
    font-size: 12px;
  }

  .info-value {
    color: var(--text);
  }

  .info-value.clickable {
    cursor: pointer;
    color: var(--blue);
  }

  .info-value.clickable:hover {
    text-decoration: underline;
  }

  /* Resource Usage */
  .resource-grid {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .resource-item {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .resource-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    color: var(--subtext0);
    font-size: 12px;
  }

  .resource-value {
    color: var(--text);
  }

  /* Progress Bar */
  .progress-bar {
    height: 6px;
    background: var(--surface1);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--blue);
    transition: width 0.2s ease;
  }

  .progress-fill.high {
    background: var(--yellow);
  }

  .progress-fill.critical {
    background: var(--red);
  }

  /* Memory and Disk Stats */
  .memory-stats,
  .disk-stats {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    color: var(--text);
  }

  /* Command and Path */
  .command-text,
  .path-text {
    word-break: break-all;
    white-space: pre-wrap;
  }

  .path-text {
    margin-top: 8px;
    font-size: 12px;
    color: var(--subtext0);
  }

  /* Process Table */
  .process-table {
    width: 100%;
    border-collapse: collapse;
  }

  .process-table th {
    text-align: left;
    padding: 8px;
    color: var(--subtext0);
    font-weight: 500;
    border-bottom: 1px solid var(--surface1);
  }

  .process-table td {
    padding: 8px;
    border-bottom: 1px solid var(--surface1);
  }

  .process-table tr:last-child td {
    border-bottom: none;
  }

  .process-table tr.clickable {
    cursor: pointer;
  }

  .process-table tr.clickable:hover {
    background: var(--surface1);
  }

  /* Environment Variables */
  .env-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 200px;
    overflow-y: auto;
    margin: -16px;
    padding: 16px;
  }

  .env-item {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
    padding: 4px 8px;
    border-radius: 4px;
    color: var(--subtext1);
    font-size: 12px;
  }

  .env-item:hover {
    background: var(--surface1);
  }

  /* Update scrollbar styles to match the container edges */
  .env-list::-webkit-scrollbar {
    width: 8px;
    height: 8px;
  }

  .env-list::-webkit-scrollbar-track {
    background: var(--surface0);
    border-radius: 0;
  }

  .env-list::-webkit-scrollbar-thumb {
    background: var(--surface2);
    border-radius: 4px;
    border: 2px solid var(--surface0);
  }

  .env-list::-webkit-scrollbar-thumb:hover {
    background: var(--surface1);
  }

  /* Scrollbar Styles */
  :global(.modal-content *::-webkit-scrollbar) {
    width: 8px;
    height: 8px;
  }

  :global(.modal-content *::-webkit-scrollbar-track) {
    background: var(--mantle);
    border-radius: 4px;
  }

  :global(.modal-content *::-webkit-scrollbar-thumb) {
    background: var(--surface2);
    border-radius: 4px;
  }

  :global(.modal-content *::-webkit-scrollbar-thumb:hover) {
    background: var(--surface1);
  }

  /* Process Control */
  .control-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .control-group + .control-group {
    margin-top: 16px;
  }

  .control-label {
    color: var(--subtext0);
    font-size: 12px;
    font-weight: 500;
  }

  .control-hint {
    color: var(--subtext0);
    font-size: 12px;
  }

  .control-hint.warning {
    color: var(--yellow);
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

  .priority-options {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 4px 12px;
  }

  .priority-option {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
  }

  .priority-option:hover {
    background: var(--surface1);
  }

  .priority-option input {
    accent-color: var(--blue);
    cursor: pointer;
  }

  .efficiency-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 13px;
  }

  .efficiency-toggle input {
    accent-color: var(--blue);
    cursor: pointer;
  }

  .core-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .core-chip {
    padding: 3px 8px;
    font-size: 11px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
    color: var(--subtext0);
    background: var(--mantle);
    border: 1px solid var(--surface1);
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .core-chip:hover:not(:disabled) {
    border-color: var(--blue);
    color: var(--text);
  }

  .core-chip.active {
    color: var(--base);
    background: var(--blue);
    border-color: var(--blue);
  }

  .core-chip:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .control-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 12px;
    margin-top: 20px;
  }

  .btn-control {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
    font-size: 13px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .btn-control:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-control.secondary {
    color: var(--text);
    background: var(--surface1);
    border: 1px solid var(--surface2);
  }

  .btn-control.secondary:hover:not(:disabled) {
    background: var(--surface2);
  }

  .btn-control.primary {
    color: var(--base);
    background: var(--blue);
    border: none;
  }

  .btn-control.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--blue) 90%, white);
  }

  .btn-control.primary.confirm {
    background: var(--yellow);
  }

  .btn-control.primary.confirm:hover:not(:disabled) {
    background: color-mix(in srgb, var(--yellow) 90%, white);
  }

  .spinner {
    width: 14px;
  }

  /* Responsive Design */
  @media (max-width: 900px) {
    .content-grid {
      grid-template-columns: 1fr;
    }

    .header-stats {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
