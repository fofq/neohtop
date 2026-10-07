<script lang="ts">
  import type { Process, Column } from "$lib/types";
  import { ProcessIcon, ActionButtons } from "$lib/components";
  import Fa from "svelte-fa";
  import {
    faChevronDown,
    faChevronRight,
  } from "@fortawesome/free-solid-svg-icons";
  import { t, statusLabel } from "$lib/i18n";

  export let process: Process;
  export let columns: Column[];
  export let isPinned: boolean;
  export let isHighUsage: boolean;
  /** True while the row flashes green after appearing in a new snapshot. */
  export let isJustStarted: boolean;
  /** True while the app has suspended this process. */
  export let isSuspended = false;
  /** Tree view: reserve the arrow slot and indent by depth. */
  export let treeMode = false;
  /** Nesting depth in the tree view (0 = root level). */
  export let depth = 0;
  export let hasChildren = false;
  export let isExpanded = false;
  /** Collapse-state key of this row (root-to-node name chain). */
  export let expandPath = "";
  /** Subtle per-family tint for the name in tree view; empty = untinted. */
  export let familyColor = "";

  export let onTogglePin: (pid: number) => void;
  export let onShowDetails: (process: Process) => void;
  export let onRestartProcess: (process: Process) => void;
  export let onToggleSuspend: (process: Process) => void;
  export let onKillProcess: (process: Process) => void;
  export let onToggleExpand: (path: string) => void = () => {};
  /** Row right-click; opens the context menu (see ProcessContextMenu). */
  export let onContextMenu: (
    process: Process,
    event: MouseEvent,
  ) => void = () => {};
  /** Row hover for the rich tooltip; skipped over the action buttons. */
  export let onHoverTooltip: (
    process: Process,
    event: MouseEvent,
  ) => void = () => {};
  export let onHideTooltip: () => void = () => {};
</script>

<tr
  class:high-usage={isHighUsage}
  class:pinned={isPinned}
  class:just-started={isJustStarted}
  class:suspended={isSuspended}
  on:contextmenu|preventDefault={(event) => onContextMenu(process, event)}
  on:dblclick={(event) => {
    // Double-click opens the details modal, except over the action
    // buttons where double clicks mean rapid button use
    const target = event.target as Element | null;
    if (target && target.closest("button, .col-actions")) return;
    onShowDetails(process);
  }}
  on:mouseenter={(event) => {
    const target = event.target as Element | null;
    if (target && target.closest(".col-actions, button")) return;
    onHoverTooltip(process, event);
  }}
  on:mouseleave={onHideTooltip}
>
  {#each columns.filter((col) => col.visible) as column}
    <td class="truncate">
      {#if column.id === "name"}
        <div
          class="name-cell"
          style={treeMode ? `padding-left: ${depth * 18}px` : undefined}
        >
          {#if treeMode}
            {#if hasChildren}
              <button
                class="expand-btn"
                on:click={() => onToggleExpand(expandPath)}
                title={isExpanded ? $t("tools.collapse") : $t("tools.expand")}
              >
                <Fa icon={isExpanded ? faChevronDown : faChevronRight} />
              </button>
            {:else}
              <span class="expand-placeholder"></span>
            {/if}
          {/if}
          <ProcessIcon processName={process.name} />
          <span
            class="process-name"
            style={familyColor ? `color: ${familyColor}` : undefined}
            title={process.name}>{process.name}</span
          >
        </div>
      {:else if column.id === "status"}
        <!-- The backend status string ("Running"/"Idle"/…) is an
             enumeration, not prose: translate it like every other
             enumerated value instead of leaking English on zh locales -->
        {$statusLabel(process.status)}
      {:else if column.format}
        {@html column.format(process[column.id])}
      {:else}
        {process[column.id]}
      {/if}
    </td>
  {/each}
  <ActionButtons
    {process}
    {isPinned}
    {isSuspended}
    {onTogglePin}
    {onShowDetails}
    {onRestartProcess}
    {onToggleSuspend}
    {onKillProcess}
  />
</tr>

<style>
  td {
    padding: 6px 12px;
    border-bottom: 1px solid var(--surface0);
    color: var(--text);
    z-index: 1;
  }

  .truncate {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 0;
  }

  tr:hover {
    background-color: var(--surface0);
  }

  .high-usage {
    background-color: color-mix(in srgb, var(--red) 10%, transparent);
  }

  .high-usage:hover {
    background-color: color-mix(in srgb, var(--red) 15%, transparent);
  }

  tr.pinned {
    background-color: color-mix(in srgb, var(--blue) 10%, transparent);
  }

  tr.pinned:hover {
    background-color: color-mix(in srgb, var(--blue) 15%, transparent);
  }

  /* New-process flash; duration is driven by the configured highlight
     duration via the --row-highlight-duration custom property. Holds a
     clearly green tint (plus a green bar on the name cell) for most of
     the window and only fades at the end, so the effect is actually
     noticeable against the auto-refresh churn. */
  tr.just-started {
    animation: startFlash var(--row-highlight-duration, 1000ms) ease-out;
  }

  tr.just-started td:first-child {
    animation: startFlashBar var(--row-highlight-duration, 1000ms) ease-out;
  }

  /* Suspended by the app: amber bar on the row start edge. */
  tr.suspended td:first-child {
    box-shadow: inset 3px 0 0 0 var(--yellow);
  }

  .expand-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--subtext0);
    cursor: pointer;
    flex-shrink: 0;
    transition: all 0.15s ease;
  }

  .expand-btn:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .expand-btn :global(svg) {
    width: 10px;
    height: 10px;
  }

  .expand-placeholder {
    width: 16px;
    flex-shrink: 0;
  }

  .name-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  @keyframes startFlash {
    0%,
    60% {
      background-color: color-mix(in srgb, var(--green) 38%, transparent);
    }
    100% {
      background-color: transparent;
    }
  }

  @keyframes startFlashBar {
    0%,
    60% {
      box-shadow: inset 3px 0 0 0 var(--green);
    }
    100% {
      box-shadow: inset 3px 0 0 0 transparent;
    }
  }
</style>
