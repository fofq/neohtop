<script lang="ts">
  import { Modal } from "$lib/components";
  import { t } from "$lib/i18n";
  import Fa from "svelte-fa";
  import { faTriangleExclamation } from "@fortawesome/free-solid-svg-icons";

  interface Process {
    pid: number;
    name: string;
  }

  export let show = false;
  export let process: Process | null = null;
  export let onClose: () => void;
  export let onConfirm: () => Promise<void>;
  export let isKilling = false;
  /**
   * Estimated tree size ("N", or "1+" when unknown) shown by the
   * kill-the-whole-tree confirmation; null = plain single kill.
   */
  export let treeCount: string | null = null;
  /**
   * Estimated application-family size shown by the kill-the-whole-app
   * confirmation; null = not an app-family kill. Takes precedence over
   * treeCount (the two scopes are mutually exclusive in the store).
   */
  export let appCount: string | null = null;

  $: scope = appCount !== null ? "app" : treeCount !== null ? "tree" : "single";
</script>

<Modal {show} title={$t("modal.confirmTitle")} maxWidth="400px" {onClose}>
  {#if process}
    <div class="confirm-content">
      <p class="confirm-message">
        {#if appCount !== null}
          <span class="tree-message">
            <Fa icon={faTriangleExclamation} />
            {$t("killApp.message", { count: appCount })}
          </span>
        {:else if treeCount !== null}
          <span class="tree-message">
            <Fa icon={faTriangleExclamation} />
            {$t("killTree.message", { count: treeCount })}
          </span>
        {:else}
          {$t("kill.message")}
        {/if}
      </p>
      <div class="process-info">
        <span class="process-name">{process.name}</span>
        <span class="process-pid">{$t("modal.pid", { pid: process.pid })}</span>
      </div>
      <div class="confirm-actions">
        <button class="btn-secondary" on:click={onClose} disabled={isKilling}>
          {$t("modal.cancel")}
        </button>
        <button class="btn-danger" on:click={onConfirm} disabled={isKilling}>
          {#if isKilling}
            <div class="spinner"></div>
            <span>
              {$t(
                scope === "app"
                  ? "killApp.inProgress"
                  : scope === "tree"
                  ? "killTree.inProgress"
                  : "kill.inProgress",
              )}
            </span>
          {:else}
            {$t(
              scope === "app"
                ? "killApp.confirm"
                : scope === "tree"
                ? "killTree.confirm"
                : "kill.confirm",
            )}
          {/if}
        </button>
      </div>
    </div>
  {/if}
</Modal>

<style>
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

  /* Tree kill: heavier warning, hierarchy via weight and a yellow icon */
  .tree-message {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
  }

  .tree-message :global(svg) {
    width: 14px;
    height: 14px;
    color: var(--yellow);
    flex-shrink: 0;
  }

  .process-info {
    background: var(--mantle);
    padding: 12px;
    border-radius: 6px;
  }

  .process-name {
    color: var(--text);
    font-weight: 500;
    font-size: 14px;
  }

  .process-pid {
    color: var(--subtext0);
    font-size: 12px;
    margin-left: 8px;
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

  .btn-danger {
    padding: 8px 16px;
    font-size: 13px;
    color: var(--base);
    background: var(--red);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .btn-danger:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .btn-danger:hover {
    background: color-mix(in srgb, var(--red) 90%, white);
  }

  .spinner {
    width: 16px;
  }
</style>
