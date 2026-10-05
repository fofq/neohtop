<script lang="ts">
  import { Modal } from "$lib/components";
  import { t } from "$lib/i18n";

  interface Process {
    pid: number;
    name: string;
  }

  export let show = false;
  export let process: Process | null = null;
  export let onClose: () => void;
  export let onConfirm: () => Promise<void>;
  export let isRestarting = false;
</script>

<Modal {show} title={$t("modal.confirmTitle")} maxWidth="400px" {onClose}>
  {#if process}
    <div class="confirm-content">
      <p class="confirm-message">{$t("restart.message")}</p>
      <div class="process-info">
        <span class="process-name">{process.name}</span>
        <span class="process-pid">{$t("modal.pid", { pid: process.pid })}</span>
      </div>
      <div class="confirm-actions">
        <button
          class="btn-secondary"
          on:click={onClose}
          disabled={isRestarting}
        >
          {$t("modal.cancel")}
        </button>
        <button
          class="btn-primary"
          on:click={onConfirm}
          disabled={isRestarting}
        >
          {#if isRestarting}
            <div class="spinner"></div>
            <span>{$t("restart.inProgress")}</span>
          {:else}
            {$t("restart.confirm")}
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

  .btn-primary:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .btn-primary:hover {
    background: color-mix(in srgb, var(--blue) 90%, white);
  }

  .spinner {
    width: 16px;
  }
</style>
