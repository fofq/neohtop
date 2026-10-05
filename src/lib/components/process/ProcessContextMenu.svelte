<script lang="ts">
  import { onMount } from "svelte";
  import Fa from "svelte-fa";
  import { faSitemap, faXmark } from "@fortawesome/free-solid-svg-icons";
  import type { Process } from "$lib/types";
  import { t } from "$lib/i18n";

  export let process: Process;
  /** Cursor position the menu opens at (viewport coordinates). */
  export let x: number;
  export let y: number;
  export let onKill: (process: Process) => void = () => {};
  export let onKillTree: (process: Process) => void = () => {};
  export let onClose: () => void = () => {};

  let menuElement: HTMLDivElement;
  let menuWidth = 0;
  let menuHeight = 0;

  // Flip at the viewport edges so the menu never clips off-screen; the
  // 4px gutter only matters for the first paint before the menu is measured.
  $: menuLeft =
    x + menuWidth > window.innerWidth ? Math.max(4, x - menuWidth) : x;
  $: menuTop =
    y + menuHeight > window.innerHeight ? Math.max(4, y - menuHeight) : y;

  // The menu closes itself before acting so the parent only runs the action
  const handleKill = () => {
    onClose();
    onKill(process);
  };

  const handleKillTree = () => {
    onClose();
    onKillTree(process);
  };

  onMount(() => {
    const closeOnOutsidePointerDown = (event: PointerEvent) => {
      if (!menuElement.contains(event.target as Node)) onClose();
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    // scroll does not bubble, but the capture phase sees every element's
    // scroll event, so the table container scrolling closes the menu too
    const scrollOptions = { capture: true, passive: true };
    window.addEventListener("pointerdown", closeOnOutsidePointerDown, true);
    window.addEventListener("keydown", closeOnEscape);
    document.addEventListener("scroll", onClose, scrollOptions);
    window.addEventListener("resize", onClose);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener(
        "pointerdown",
        closeOnOutsidePointerDown,
        true,
      );
      window.removeEventListener("keydown", closeOnEscape);
      document.removeEventListener("scroll", onClose, scrollOptions);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("blur", onClose);
    };
  });
</script>

<div
  bind:this={menuElement}
  bind:clientWidth={menuWidth}
  bind:clientHeight={menuHeight}
  class="context-menu"
  role="menu"
  tabindex="-1"
  style="left: {menuLeft}px; top: {menuTop}px;"
  on:contextmenu|preventDefault
>
  <button class="menu-item danger" role="menuitem" on:click={handleKill}>
    <Fa icon={faXmark} />
    <span>{$t("contextMenu.endProcess")}</span>
  </button>
  <button class="menu-item danger" role="menuitem" on:click={handleKillTree}>
    <Fa icon={faSitemap} />
    <span>{$t("contextMenu.endProcessTree")}</span>
  </button>
</div>

<style>
  .context-menu {
    position: fixed;
    /* Above the hover card (900) and the sticky actions column, below the
       modal backdrop (1000) so a confirm modal always covers the menu */
    z-index: 950;
    /* Hugs the content so future entries reshape the menu instead of
       leaving dead space */
    width: max-content;
    min-width: 132px;
    max-width: 320px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    background: var(--base);
    border: 1px solid var(--surface1);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    text-align: left;
    white-space: nowrap;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .menu-item:hover {
    background: var(--surface0);
  }

  .menu-item.danger {
    color: var(--red);
  }

  .menu-item.danger:hover {
    background: color-mix(in srgb, var(--red) 12%, transparent);
  }

  .menu-item :global(svg) {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
  }
</style>
