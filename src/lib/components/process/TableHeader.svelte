<script lang="ts">
  import type { Process, Column } from "$lib/types";
  import { t } from "$lib/i18n";

  export let columns: Column[];
  export let sortConfig: { field: keyof Process; direction: "asc" | "desc" };
  export let onToggleSort: (field: keyof Process) => void;
  /** Live column resize (px) while a handle is dragged. */
  export let onColumnResize: (id: string, width: number) => void = () => {};
  /** Persists widths once the drag gesture ends. */
  export let onColumnResizeEnd: () => void = () => {};
  /** Double-click on the name handle: re-fit to the widest visible name. */
  export let onAutoFitName: () => void = () => {};

  let draggingId: string | null = null;
  let dragStartX = 0;
  let dragStartWidth = 0;

  const RESIZE_MIN_WIDTH = 80;
  const RESIZE_MAX_WIDTH = 1000;

  function handleResizeStart(event: MouseEvent, id: string) {
    event.preventDefault();
    event.stopPropagation();
    const handle = event.currentTarget as HTMLElement;
    const th = handle.closest("th");
    draggingId = id;
    dragStartX = event.pageX;
    // Start from the rendered width so never-resized columns (whose width
    // the browser chose) resize from where they actually are.
    dragStartWidth = th ? th.getBoundingClientRect().width : RESIZE_MIN_WIDTH;
    // Keep the gesture from selecting text or flipping the sort order.
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", handleResizeMove);
    window.addEventListener("mouseup", handleResizeEnd);
  }

  function handleResizeMove(event: MouseEvent) {
    if (!draggingId) return;
    const width = Math.min(
      RESIZE_MAX_WIDTH,
      Math.max(RESIZE_MIN_WIDTH, dragStartWidth + (event.pageX - dragStartX)),
    );
    onColumnResize(draggingId, width);
  }

  function handleResizeEnd() {
    if (!draggingId) return;
    draggingId = null;
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    window.removeEventListener("mousemove", handleResizeMove);
    window.removeEventListener("mouseup", handleResizeEnd);
    onColumnResizeEnd();
  }

  function getSortIndicator(field: keyof Process) {
    if (sortConfig.field !== field) return "↕";
    return sortConfig.direction === "asc" ? "↑" : "↓";
  }
</script>

<thead>
  <tr>
    {#each columns.filter((col) => col.visible) as column}
      <th class="sortable" on:click={() => onToggleSort(column.id)}>
        <div class="th-content">
          {$t("col." + column.id)}
          <span
            class="sort-indicator"
            class:active={sortConfig.field === column.id}
          >
            {getSortIndicator(column.id)}
          </span>
        </div>
        <!-- Column resize affordance: pointer-only by design (drag /
             double-click); sort clicks are stopped so they never flip the
             column's sort order. -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <span
          class="resize-handle"
          class:dragging={draggingId === column.id}
          role="separator"
          aria-orientation="vertical"
          title={$t("table.resizeColumn")}
          on:mousedown={(event) => handleResizeStart(event, column.id)}
          on:dblclick|stopPropagation={() =>
            column.id === "name" && onAutoFitName()}
          on:click|stopPropagation
        ></span>
      </th>
    {/each}
    <th>{$t("table.actions")}</th>
  </tr>
</thead>

<style>
  th {
    position: sticky;
    top: 0;
    background: var(--mantle);
    text-align: left;
    padding: 8px 12px;
    font-weight: 500;
    color: var(--subtext0);
    border-bottom: 1px solid var(--surface0);
    transition: background-color 0.2s ease;
    z-index: 3;
  }

  th:last-child {
    width: 190px;
    min-width: 190px;
    max-width: 190px;
  }

  .sortable {
    cursor: pointer;
    user-select: none;
  }

  .th-content {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .sort-indicator {
    color: var(--overlay0);
    font-size: 12px;
    opacity: 0.5;
    transition: all 0.2s ease;
  }

  .sort-indicator.active {
    color: var(--blue);
    opacity: 1;
  }

  .sortable:hover .sort-indicator {
    opacity: 1;
  }

  /* Column resize handle: a 6px hit zone on the header's right edge with
     a 2px guide line that lights up on hover and while dragging. */
  .resize-handle {
    position: absolute;
    top: 0;
    right: 0;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 4;
  }

  .resize-handle::after {
    content: "";
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 2px;
    background: transparent;
    transition: background-color 0.15s ease;
  }

  .resize-handle:hover::after {
    background: var(--surface2);
  }

  .resize-handle.dragging::after {
    background: var(--blue);
  }
</style>
