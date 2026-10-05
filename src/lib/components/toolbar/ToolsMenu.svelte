<script lang="ts">
  import Fa from "svelte-fa";
  import {
    faFileCircleQuestion,
    faGears,
    faRocket,
    faToolbox,
    faWindowRestore,
  } from "@fortawesome/free-solid-svg-icons";
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { overlayStore } from "$lib/stores/overlay";

  export let onShowFileLockers: () => void;
  export let onShowServices: () => void;
  export let onShowWindows: () => void;
  export let onShowStartupItems: () => void;

  let containerElement: HTMLDivElement;
  let panelElement: HTMLDivElement;

  $: showMenu = $overlayStore === "tools";

  /**
   * Renders the panel as a direct child of <body>. The toolbar is a flex
   * container that can wrap at the window's minimum width; keeping the
   * fixed panel inside it leaves it losing stacking/geometry races
   * against the table below. A body-level fixed element always wins.
   */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      },
    };
  }

  function openFileLockers() {
    overlayStore.close();
    onShowFileLockers();
  }

  function openServices() {
    overlayStore.close();
    onShowServices();
  }

  function openWindows() {
    overlayStore.close();
    onShowWindows();
  }

  function openStartupItems() {
    overlayStore.close();
    onShowStartupItems();
  }

  function toggleMenu(event: Event) {
    event.stopPropagation();
    if (showMenu) {
      overlayStore.close();
    } else {
      overlayStore.open("tools");
      setTimeout(updatePanelPosition, 0);
    }
  }

  // Anchors the panel below the button, LEFT-aligned to it; flips to
  // right-anchoring only when the panel would overflow the window's right
  // edge. Left-alignment is what keeps the panel on-screen when the
  // wrapped toolbar puts the button near the window's left edge.
  function updatePanelPosition() {
    if (containerElement && panelElement) {
      const rect = containerElement.getBoundingClientRect();
      panelElement.style.top = `${rect.bottom + 6}px`;
      const width = panelElement.offsetWidth || 180;
      if (rect.left + width > window.innerWidth - 4) {
        panelElement.style.left = "auto";
        panelElement.style.right = `${Math.max(
          4,
          window.innerWidth - rect.right,
        )}px`;
      } else {
        panelElement.style.left = `${Math.max(4, rect.left)}px`;
        panelElement.style.right = "auto";
      }
    }
  }

  function handleClickOutside(event: MouseEvent) {
    if (
      showMenu &&
      containerElement &&
      panelElement &&
      !containerElement.contains(event.target as Node) &&
      !panelElement.contains(event.target as Node)
    ) {
      overlayStore.close();
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && showMenu) {
      overlayStore.close();
    }
  }

  function setupListeners() {
    if (typeof document !== "undefined") {
      document.addEventListener("click", handleClickOutside);
      document.addEventListener("keydown", handleKeydown);
    }
  }

  function cleanupListeners() {
    if (typeof document !== "undefined") {
      document.removeEventListener("click", handleClickOutside);
      document.removeEventListener("keydown", handleKeydown);
    }
  }

  // Attach/detach document listeners only when visibility flips, so the
  // initial closed state doesn't trigger a needless cleanup.
  let wasOpen = false;
  $: if (showMenu !== wasOpen) {
    wasOpen = showMenu;
    if (showMenu) {
      setTimeout(setupListeners, 0);
    } else {
      cleanupListeners();
    }
  }

  onDestroy(cleanupListeners);
</script>

<svelte:window on:resize={() => showMenu && updatePanelPosition()} />

<div class="tools-menu" bind:this={containerElement}>
  <button
    class="tools-button"
    class:active={showMenu}
    on:click={toggleMenu}
    aria-label={$t("tools.ariaToggle")}
    aria-expanded={showMenu}
    aria-haspopup="dialog"
    title={$t("tools.menu")}
  >
    <Fa icon={faToolbox} />
  </button>

  {#if showMenu}
    <div
      class="tools-panel"
      bind:this={panelElement}
      use:portal
      role="dialog"
      aria-label={$t("tools.menu")}
      tabindex="-1"
    >
      <button class="menu-option" on:click={openFileLockers}>
        <span class="option-icon"><Fa icon={faFileCircleQuestion} /></span>
        <span class="option-label">{$t("tools.fileLockers")}</span>
      </button>
      <button class="menu-option" on:click={openServices}>
        <span class="option-icon"><Fa icon={faGears} /></span>
        <span class="option-label">{$t("tools.services")}</span>
      </button>
      <button class="menu-option" on:click={openStartupItems}>
        <span class="option-icon"><Fa icon={faRocket} /></span>
        <span class="option-label">{$t("tools.startupItems")}</span>
      </button>
      <button class="menu-option" on:click={openWindows}>
        <span class="option-icon"><Fa icon={faWindowRestore} /></span>
        <span class="option-label">{$t("tools.windows")}</span>
      </button>
    </div>
  {/if}
</div>

<style>
  .tools-menu {
    position: relative;
    display: flex;
    align-items: center;
    margin-right: 8px;
  }

  .tools-button {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 28px;
    width: 34px;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    box-sizing: border-box;
    padding: 0;
  }

  .tools-button:hover,
  .tools-button.active {
    color: var(--text);
    background: var(--surface1);
    border-color: var(--blue);
  }

  .tools-panel {
    position: fixed;
    z-index: 1100;
    min-width: 180px;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--mantle);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
    animation: panelSlideIn 0.15s ease-out;
    animation-fill-mode: both;
  }

  .menu-option {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text);
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
    text-align: left;
    box-sizing: border-box;
  }

  .menu-option:hover {
    background: var(--surface0);
  }

  .option-icon {
    display: inline-flex;
    width: 14px;
    justify-content: center;
    color: var(--subtext0);
  }

  .option-icon :global(svg) {
    font-size: 11px;
  }

  @keyframes panelSlideIn {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
</style>
