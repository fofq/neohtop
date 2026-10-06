<script lang="ts">
  import Fa from "svelte-fa";
  import { faGear, faShieldHalved } from "@fortawesome/free-solid-svg-icons";
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { languageSetting, locales, t, type LanguageSetting } from "$lib/i18n";
  import { settingsStore, isElevated } from "$lib/stores/index";
  import { overlayStore } from "$lib/stores/overlay";
  import { HIGHLIGHT_DURATION_OPTIONS } from "$lib/constants";
  import { Modal } from "$lib/components";

  interface MenuOption {
    key: string;
    label: string;
    selected: boolean;
    onSelect: () => void;
  }

  interface MenuSection {
    title: string;
    options: MenuOption[];
  }

  let containerElement: HTMLDivElement;
  let panelElement: HTMLDivElement;

  $: showMenu = $overlayStore === "settings";

  function selectLanguage(value: LanguageSetting) {
    languageSetting.set(value);
    settingsStore.updateConfig({ language: value });
    overlayStore.close();
  }

  function selectHighlighting(enabled: boolean) {
    settingsStore.updateConfig({
      appearance: {
        ...$settingsStore.appearance,
        highlighting: {
          ...$settingsStore.appearance.highlighting,
          enabled,
        },
      },
    });
  }

  function selectHighlightDuration(durationMs: number) {
    settingsStore.updateConfig({
      appearance: {
        ...$settingsStore.appearance,
        highlighting: {
          ...$settingsStore.appearance.highlighting,
          durationMs,
        },
      },
    });
  }

  // Sections are data-driven; add a new entry here to extend the menu.
  // Highlight options keep the panel open so toggle and duration can be
  // adjusted in one pass; the radio indicator gives immediate feedback.
  let sections: MenuSection[];
  $: sections = [
    {
      title: $t("settings.language"),
      options: [
        {
          key: "auto",
          label: $t("settings.autoLanguage"),
          selected: $languageSetting === "auto",
          onSelect: () => selectLanguage("auto"),
        },
        ...locales.map((l) => ({
          key: l.value,
          label: l.label,
          selected: $languageSetting === l.value,
          onSelect: () => selectLanguage(l.value),
        })),
      ],
    },
    {
      title: $t("settings.highlighting"),
      options: [
        {
          key: "enabled",
          label: $t("settings.highlightOn"),
          selected: $settingsStore.appearance.highlighting.enabled,
          onSelect: () => selectHighlighting(true),
        },
        {
          key: "disabled",
          label: $t("settings.highlightOff"),
          selected: !$settingsStore.appearance.highlighting.enabled,
          onSelect: () => selectHighlighting(false),
        },
      ],
    },
    {
      title: $t("settings.highlightDuration"),
      options: HIGHLIGHT_DURATION_OPTIONS.map((option) => ({
        key: String(option.value),
        label: option.label,
        selected:
          $settingsStore.appearance.highlighting.durationMs === option.value,
        onSelect: () => selectHighlightDuration(option.value),
      })),
    },
  ];

  function toggleMenu(event: Event) {
    event.stopPropagation();
    if (showMenu) {
      overlayStore.close();
    } else {
      overlayStore.open("settings");
      setTimeout(updatePanelPosition, 0);
    }
  }

  // --- Elevation (administrator) section state ---
  let showElevationConfirm = false;
  let isRestartingAsAdmin = false;
  // Set once the elevated instance has been launched and this one is exiting
  let elevationError: string | null = null;

  function confirmRestartAsAdmin() {
    elevationError = null;
    // Close the panel first: it stacks above the modal backdrop (z-index)
    overlayStore.close();
    showElevationConfirm = true;
  }

  async function restartAsAdmin() {
    isRestartingAsAdmin = true;
    elevationError = null;
    try {
      const launched = await invoke<boolean>("restart_as_admin");
      showElevationConfirm = false;
      if (launched) {
        // The relaunch dialog closes; the elevated instance takes over
      }
    } catch (e: unknown) {
      elevationError = e instanceof Error ? e.message : String(e);
    } finally {
      isRestartingAsAdmin = false;
    }
  }

  // The title bar clips overflow, so the panel is fixed-positioned
  // relative to the viewport and anchored to the gear button.
  function updatePanelPosition() {
    if (containerElement && panelElement) {
      const rect = containerElement.getBoundingClientRect();
      panelElement.style.top = `${rect.bottom + 6}px`;
      panelElement.style.right = `${window.innerWidth - rect.right}px`;
    }
  }

  function handleClickOutside(event: MouseEvent) {
    if (
      showMenu &&
      containerElement &&
      !containerElement.contains(event.target as Node)
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

<div class="settings-menu" bind:this={containerElement}>
  <!-- Elevation shortcut: a one-glance status that turns green once the app
       runs as admin; clicking relaunches elevated (confirm modal below).
       Replaces the old settings-panel section — same flow, one less click. -->
  {#if $isElevated}
    <span
      class="settings-button shield-button is-elevated"
      role="status"
      title={$t("settings.elevationRunning")}
    >
      <Fa icon={faShieldHalved} />
    </span>
  {:else}
    <button
      class="settings-button shield-button"
      on:click={confirmRestartAsAdmin}
      disabled={isRestartingAsAdmin}
      title={$t("settings.elevationDescription")}
      aria-label={$t("settings.elevationAction")}
    >
      <Fa icon={faShieldHalved} />
    </button>
  {/if}
  <button
    class="settings-button"
    class:active={showMenu}
    on:click={toggleMenu}
    aria-label={$t("settings.ariaToggle")}
    aria-expanded={showMenu}
    aria-haspopup="dialog"
  >
    <Fa icon={faGear} />
  </button>

  {#if showMenu}
    <div
      class="settings-panel"
      bind:this={panelElement}
      role="dialog"
      aria-label={$t("settings.title")}
      tabindex="-1"
    >
      <div class="panel-title">{$t("settings.title")}</div>
      {#each sections as section (section.title)}
        <div class="menu-section">
          <div class="section-label">{section.title}</div>
          <div
            class="section-options"
            role="radiogroup"
            aria-label={section.title}
          >
            {#each section.options as option (option.key)}
              <button
                class="menu-option"
                class:selected={option.selected}
                role="radio"
                aria-checked={option.selected}
                on:click={option.onSelect}
              >
                <span class="radio-indicator"></span>
                <span class="option-label">{option.label}</span>
              </button>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<Modal
  show={showElevationConfirm}
  title={$t("settings.elevationConfirmTitle")}
  maxWidth="400px"
  onClose={() => (showElevationConfirm = false)}
>
  <div class="confirm-content">
    <p class="confirm-message">{$t("settings.elevationConfirmMessage")}</p>
    {#if elevationError}
      <p class="confirm-error">{elevationError}</p>
    {/if}
    <div class="confirm-actions">
      <button
        class="btn-secondary"
        on:click={() => (showElevationConfirm = false)}
        disabled={isRestartingAsAdmin}
      >
        {$t("modal.cancel")}
      </button>
      <button
        class="btn-primary"
        on:click={restartAsAdmin}
        disabled={isRestartingAsAdmin}
      >
        {#if isRestartingAsAdmin}
          <div class="spinner"></div>
          <span>{$t("settings.elevationRelaunching")}</span>
        {:else}
          <Fa icon={faShieldHalved} />
          <span>{$t("settings.elevationConfirm")}</span>
        {/if}
      </button>
    </div>
  </div>
</Modal>

<style>
  .settings-menu {
    position: absolute;
    right: 12px;
    top: 50%;
    display: flex;
    gap: 6px;
    align-items: center;
    /* Centered via negative margin, not transform: a transformed ancestor
       becomes the containing block for position:fixed descendants, which
       would anchor the panel to this box and let the title bar clip it. */
    margin-top: -12px;
  }

  /* Shield shortcut: muted until elevated, green when admin is active */
  .shield-button {
    color: var(--subtext0);
  }

  .shield-button:hover {
    color: var(--yellow);
    border-color: var(--yellow);
  }

  .shield-button.is-elevated {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 12%, transparent);
    border-color: color-mix(in srgb, var(--green) 40%, transparent);
    cursor: default;
  }

  .settings-button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
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

  .settings-button:hover {
    color: var(--text);
    background: var(--surface1);
    border-color: var(--blue);
  }

  .settings-button.active {
    color: var(--text);
    background: var(--surface1);
    border-color: var(--blue);
  }

  .settings-panel {
    position: fixed;
    z-index: 1100;
    min-width: 200px;
    padding: 8px;
    background: var(--mantle);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
    animation: panelSlideIn 0.15s ease-out;
    animation-fill-mode: both;
  }

  .panel-title {
    padding: 0 10px 6px;
    font-size: 12px;
    font-weight: bold;
    color: var(--text);
  }

  .menu-section + .menu-section {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--surface0);
  }

  .section-label {
    padding: 0 10px 4px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.4px;
    text-transform: uppercase;
    color: var(--subtext0);
  }

  .section-options {
    display: flex;
    flex-direction: column;
    gap: 2px;
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

  .menu-option.selected {
    color: var(--blue);
  }

  .radio-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    border: 1px solid var(--overlay0);
    border-radius: 50%;
    box-sizing: border-box;
    flex-shrink: 0;
    transition: all 0.15s ease;
  }

  .menu-option.selected .radio-indicator {
    border-color: var(--blue);
  }

  .menu-option.selected .radio-indicator::after {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--blue);
  }

  .confirm-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .confirm-message {
    color: var(--text);
    margin: 0;
    font-size: 14px;
  }

  .confirm-error {
    color: var(--red);
    margin: 0;
    font-size: 12px;
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
  }

  .btn-secondary {
    padding: 8px 16px;
    font-size: 13px;
    color: var(--text);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .btn-secondary:hover {
    background: var(--surface1);
  }

  .btn-primary {
    padding: 8px 16px;
    font-size: 13px;
    color: var(--base);
    background: var(--blue);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .btn-primary:hover {
    background: color-mix(in srgb, var(--blue) 90%, white);
  }

  .btn-primary:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .spinner {
    width: 16px;
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
