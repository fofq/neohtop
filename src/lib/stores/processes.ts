import { writable, derived, get } from "svelte/store";
import type { PerformanceSample, Process, SystemStats } from "$lib/types";
import { invoke } from "@tauri-apps/api/core";
import { t } from "$lib/i18n";
import { settingsStore } from "./settings";
import {
  countProcessTreeSize,
  countAppFamilySize,
  withElevationHint,
} from "$lib/utils";

interface ProcessStore {
  processes: Process[];
  systemStats: SystemStats | null;
  error: string | null;
  isLoading: boolean;
  searchTerm: string;
  currentPage: number;
  pinnedProcesses: Set<number>;
  selectedProcess: Process | null;
  showInfoModal: boolean;
  showConfirmModal: boolean;
  processToKill: Process | null;
  isKilling: boolean;
  /**
   * Estimated tree size for the kill-tree confirmation ("N", or "1+" when
   * the root is missing from the snapshot); null = plain single kill.
   */
  killTreeCount: string | null;
  /**
   * Estimated application-family size for the kill-app confirmation ("N",
   * or "1+" when the target is missing); null = not an app-family kill.
   */
  killAppCount: string | null;
  /** Transient success notice (e.g. the kill-tree result); auto-clears. */
  notice: string | null;
  showRestartModal: boolean;
  processToRestart: Process | null;
  isRestarting: boolean;
  /** PIDs the app itself has suspended (suspend_process succeeded). */
  suspendedPids: Set<number>;
  isFrozen: boolean;
  selectedProcessPid: number | null;
  /** PIDs that appeared in the latest snapshot (highlight flash). */
  justStarted: Set<number>;
  /** PIDs that disappeared in the latest snapshot. */
  justExited: Set<number>;
  /**
   * Performance ring buffer of the selected process, fed once per polling
   * cycle while it stays selected; empty when nothing is selected.
   */
  selectedHistory: PerformanceHistory;
  sortConfig: {
    field: keyof Process;
    direction: "asc" | "desc";
  };
}

/**
 * Ring buffer of performance samples for the details modal charts. The
 * disk fields of the samples are per-interval deltas; `lastDiskTotals`
 * keeps the raw counters of the previous snapshot to compute them.
 */
interface PerformanceHistory {
  pid: number | null;
  points: PerformanceSample[];
  lastDiskTotals: { read: number; write: number } | null;
}

const emptyHistory = (pid: number | null = null): PerformanceHistory => ({
  pid,
  points: [],
  lastDiskTotals: null,
});

/** How many samples the performance charts keep. */
const HISTORY_LENGTH = 120;

/** How long the success notice stays on screen. */
const NOTICE_DURATION_MS = 4000;

const initialState: ProcessStore = {
  processes: [],
  systemStats: null,
  error: null,
  isLoading: true,
  searchTerm: "",
  currentPage: 1,
  pinnedProcesses: new Set(),
  selectedProcess: null,
  showInfoModal: false,
  showConfirmModal: false,
  processToKill: null,
  isKilling: false,
  killTreeCount: null,
  killAppCount: null,
  notice: null,
  showRestartModal: false,
  processToRestart: null,
  isRestarting: false,
  suspendedPids: new Set(),
  isFrozen: false,
  selectedProcessPid: null,
  justStarted: new Set(),
  justExited: new Set(),
  selectedHistory: emptyHistory(),
  sortConfig: {
    field: "cpu_usage",
    direction: "desc",
  },
};

function createProcessStore() {
  const { subscribe, set, update } = writable<ProcessStore>(initialState);

  // Define all methods first
  const setIsLoading = (isLoading: boolean) =>
    update((state) => ({ ...state, isLoading }));

  // Transient success feedback, mirroring the error alert above the table;
  // a newer notice replaces the pending auto-clear of the previous one.
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;
  const showNotice = (message: string) => {
    if (noticeTimer !== null) clearTimeout(noticeTimer);
    update((state) => ({ ...state, notice: message }));
    noticeTimer = setTimeout(() => {
      noticeTimer = null;
      update((state) => ({ ...state, notice: null }));
    }, NOTICE_DURATION_MS);
  };

  // --- New/exited process highlight tracking ---
  // Each snapshot replaces the list wholesale, so diffing is plain Set math
  // over the previous frame's PIDs; highlights expire on a timer matching
  // the configured duration.
  const clearHighlightsAfter = (
    started: Set<number>,
    exited: Set<number>,
    durationMs: number,
  ) => {
    if (started.size === 0 && exited.size === 0) return;
    setTimeout(() => {
      update((state) => {
        const justStarted = new Set(state.justStarted);
        const justExited = new Set(state.justExited);
        let expired = false;
        for (const pid of started) {
          if (justStarted.delete(pid)) expired = true;
        }
        for (const pid of exited) {
          if (justExited.delete(pid)) expired = true;
        }
        // Only emit when something actually expired to avoid needless updates.
        return expired ? { ...state, justStarted, justExited } : state;
      });
    }, durationMs);
  };

  const diffHighlights = (
    prevProcesses: Process[],
    nextProcesses: Process[],
    prevStarted: Set<number>,
    prevExited: Set<number>,
  ): { justStarted: Set<number>; justExited: Set<number> } => {
    const highlighting = get(settingsStore).appearance.highlighting;
    if (!highlighting?.enabled) {
      return { justStarted: new Set(), justExited: new Set() };
    }
    if (prevProcesses.length === 0) {
      // First snapshot (or first one after an error): nothing to diff
      // against, so don't flash the entire table as newly started.
      return { justStarted: prevStarted, justExited: prevExited };
    }
    const prevPids = new Set(prevProcesses.map((p) => p.pid));
    const nextPids = new Set(nextProcesses.map((p) => p.pid));
    const started = new Set<number>();
    const exited = new Set<number>();
    for (const pid of nextPids) {
      if (!prevPids.has(pid)) started.add(pid);
    }
    for (const pid of prevPids) {
      if (!nextPids.has(pid)) exited.add(pid);
    }
    // Merge into copies so highlights from a previous frame keep decaying
    // on their own timers even if a new snapshot adds more.
    const justStarted = new Set(prevStarted);
    const justExited = new Set(prevExited);
    for (const pid of started) justStarted.add(pid);
    for (const pid of exited) justExited.add(pid);
    clearHighlightsAfter(started, exited, highlighting.durationMs);
    return { justStarted, justExited };
  };

  // Pushes one performance sample for the selected process, keeping the
  // last HISTORY_LENGTH points. Disk counters are cumulative totals from
  // sysinfo, so the chart values are deltas against the previous snapshot;
  // a dropped or reused PID resets the buffer so data never mixes.
  const recordHistorySample = (
    history: PerformanceHistory,
    pid: number | null,
    processes: Process[],
  ): PerformanceHistory => {
    if (pid === null || history.pid !== pid) return emptyHistory(pid);
    const process = processes.find((p) => p.pid === pid);
    if (!process) return emptyHistory(pid);
    const lastTotals = history.lastDiskTotals;
    const [read, write] = process.disk_usage;
    const point: PerformanceSample = {
      cpu: process.cpu_usage,
      memory: process.memory_usage,
      disk_read: lastTotals ? Math.max(0, read - lastTotals.read) : 0,
      disk_write: lastTotals ? Math.max(0, write - lastTotals.write) : 0,
    };
    const points = [...history.points, point].slice(-HISTORY_LENGTH);
    return { pid, points, lastDiskTotals: { read, write } };
  };

  const getProcesses = async () => {
    try {
      const result = await invoke<[Process[], SystemStats]>("get_processes");
      update((state) => {
        let updatedSelectedProcess = state.selectedProcess;
        if (state.selectedProcessPid) {
          updatedSelectedProcess =
            result[0].find((p) => p.pid === state.selectedProcessPid) || null;
        }

        const { justStarted, justExited } = diffHighlights(
          state.processes,
          result[0],
          state.justStarted,
          state.justExited,
        );

        // Drop suspend markers for processes that are gone so a reused
        // PID never shows a stale suspended state.
        const suspendedPids = new Set(state.suspendedPids);
        for (const pid of suspendedPids) {
          if (!result[0].some((p) => p.pid === pid)) suspendedPids.delete(pid);
        }

        // Same for pins: a pin on an exited process would otherwise float
        // an unrelated process to the top once the PID gets reused.
        const pinnedProcesses = new Set(state.pinnedProcesses);
        for (const pid of pinnedProcesses) {
          if (!result[0].some((p) => p.pid === pid))
            pinnedProcesses.delete(pid);
        }

        return {
          ...state,
          processes: result[0],
          systemStats: result[1],
          error: null,
          selectedProcess: updatedSelectedProcess,
          justStarted,
          justExited,
          suspendedPids,
          pinnedProcesses,
          selectedHistory: recordHistorySample(
            state.selectedHistory,
            state.selectedProcessPid,
            result[0],
          ),
        };
      });
    } catch (e: unknown) {
      update((state) => ({
        ...state,
        error: withElevationHint(e instanceof Error ? e.message : String(e)),
      }));
    }
  };

  const killProcess = async (pid: number) => {
    try {
      update((state) => ({ ...state, isKilling: true }));
      const success = await invoke<boolean>("kill_process", { pid });
      if (success) {
        await getProcesses();
      } else {
        throw new Error("Failed to kill process");
      }
    } catch (e: unknown) {
      update((state) => ({
        ...state,
        error: withElevationHint(e instanceof Error ? e.message : String(e)),
      }));
    } finally {
      update((state) => ({ ...state, isKilling: false }));
    }
  };

  // Kills the whole tree rooted at pid. The backend reports what it
  // collected versus what actually died; the notice shows both counts, and
  // a run where not even one process died goes down the error path (its
  // most likely cause is missing elevation, so the hint applies).
  const killProcessTree = async (pid: number) => {
    try {
      update((state) => ({ ...state, isKilling: true }));
      const result = await invoke<{ requested: number; killed: number }>(
        "kill_process_tree",
        { pid },
      );
      if (result.killed === 0) {
        throw new Error("Failed to kill process tree");
      }
      showNotice(
        get(t)("killTree.success", {
          killed: result.killed,
          requested: result.requested,
        }),
      );
      await getProcesses();
    } catch (e: unknown) {
      update((state) => ({
        ...state,
        error: withElevationHint(e instanceof Error ? e.message : String(e)),
      }));
    } finally {
      update((state) => ({ ...state, isKilling: false }));
    }
  };

  // Kills the whole application of pid (Task Manager's "end task").
  // The backend resolves the application (the target plus its
  // same-executable descendants) and reports requested/killed plus any
  // PIDs a supervisor relaunched in the brief re-scan; those are surfaced
  // as a follow-on notice, not chased automatically.
  const killAppProcess = async (pid: number) => {
    try {
      update((state) => ({ ...state, isKilling: true }));
      const result = await invoke<{
        requested: number;
        killed: number;
        respawns: number[];
      }>("kill_app_family", { pid });
      if (result.killed === 0) {
        throw new Error("Failed to kill application");
      }
      // showNotice replaces the previous notice, so the kill tally and the
      // respawn follow-up are merged into one message
      let message = get(t)("killApp.success", {
        killed: result.killed,
        requested: result.requested,
      });
      if (result.respawns.length > 0) {
        message += ` ${get(t)("killApp.respawns", {
          count: result.respawns.length,
          pids: result.respawns
            .slice(0, 4)
            .join(", ")
            .concat(result.respawns.length > 4 ? ", …" : ""),
        })}`;
      }
      showNotice(message);
      await getProcesses();
    } catch (e: unknown) {
      update((state) => ({
        ...state,
        error: withElevationHint(e instanceof Error ? e.message : String(e)),
      }));
    } finally {
      update((state) => ({ ...state, isKilling: false }));
    }
  };

  const restartProcess = async (pid: number) => {
    try {
      update((state) => ({ ...state, isRestarting: true }));
      const success = await invoke<boolean>("restart_process", { pid });
      if (success) {
        await getProcesses();
      } else {
        throw new Error("Failed to restart process");
      }
    } catch (e: unknown) {
      update((state) => ({
        ...state,
        error: withElevationHint(e instanceof Error ? e.message : String(e)),
      }));
    } finally {
      update((state) => ({ ...state, isRestarting: false }));
    }
  };

  // Suspend/resume toggle. Both actions are gentle (fully reversible), so
  // unlike kill/restart they run without a confirmation modal; the app
  // tracks the suspended state itself because the backend process list
  // cannot report it (sysinfo always reports "Running" on Windows).
  const toggleSuspend = async (process: Process) => {
    const pid = process.pid;
    let isSuspended = false;
    const unsubscribe = subscribe((state) => {
      isSuspended = state.suspendedPids.has(pid);
    });
    unsubscribe();

    try {
      const command = isSuspended ? "resume_process" : "suspend_process";
      const success = await invoke<boolean>(command, { pid });
      if (!success) {
        throw new Error(
          isSuspended
            ? "Failed to resume process"
            : "Failed to suspend process",
        );
      }
      update((state) => {
        const suspendedPids = new Set(state.suspendedPids);
        if (isSuspended) {
          suspendedPids.delete(pid);
        } else {
          suspendedPids.add(pid);
        }
        return { ...state, suspendedPids };
      });
    } catch (e: unknown) {
      update((state) => ({
        ...state,
        error: withElevationHint(e instanceof Error ? e.message : String(e)),
      }));
    }
  };

  const toggleSort = (field: keyof Process) => {
    update((state) => ({
      ...state,
      sortConfig: {
        field,
        direction:
          state.sortConfig.field === field
            ? state.sortConfig.direction === "asc"
              ? "desc"
              : "asc"
            : "desc",
      },
    }));
  };

  const togglePin = (pid: number) => {
    update((state) => {
      const newPinnedProcesses = new Set(state.pinnedProcesses);
      if (newPinnedProcesses.has(pid)) {
        newPinnedProcesses.delete(pid);
      } else {
        newPinnedProcesses.add(pid);
      }
      return { ...state, pinnedProcesses: newPinnedProcesses };
    });
  };

  const setSearchTerm = (searchTerm: string) =>
    update((state) => ({ ...state, searchTerm, currentPage: 1 }));

  const setIsFrozen = (isFrozen: boolean) =>
    update((state) => ({ ...state, isFrozen }));

  const setCurrentPage = (currentPage: number) =>
    update((state) => ({ ...state, currentPage }));

  const showProcessDetails = (process: Process) => {
    update((state) => ({
      ...state,
      selectedProcessPid: process.pid,
      selectedProcess: process,
      showInfoModal: true,
      // Restart the charts when switching to another process; keep the
      // buffer when the same process is reopened so its history survives.
      selectedHistory:
        state.selectedHistory.pid === process.pid
          ? state.selectedHistory
          : emptyHistory(process.pid),
    }));
  };

  const closeProcessDetails = () => {
    update((state) => ({
      ...state,
      showInfoModal: false,
      selectedProcess: null,
      selectedProcessPid: null,
      selectedHistory: emptyHistory(),
    }));
  };

  const confirmKillProcess = (process: Process) => {
    update((state) => ({
      ...state,
      processToKill: process,
      showConfirmModal: true,
      killTreeCount: null,
      killAppCount: null,
    }));
  };

  // Same confirm modal as the plain kill, switched to the tree warning by
  // the estimated descendant count taken from the current snapshot
  const confirmKillTreeProcess = (process: Process) => {
    update((state) => {
      const treeSize = countProcessTreeSize(state.processes, process.pid);
      return {
        ...state,
        processToKill: process,
        showConfirmModal: true,
        // A root missing from the snapshot gets an open-ended count
        killTreeCount: treeSize === null ? "1+" : String(treeSize),
        killAppCount: null,
      };
    });
  };

  // Same confirm modal as the plain kill, switched to the app-family
  // warning by the estimated family size taken from the current snapshot
  const confirmKillAppProcess = (process: Process) => {
    update((state) => {
      const size = countAppFamilySize(state.processes, process.pid);
      return {
        ...state,
        processToKill: process,
        showConfirmModal: true,
        killTreeCount: null,
        // A target missing from the snapshot gets an open-ended count
        killAppCount: size === null ? "1+" : String(size),
      };
    });
  };

  const closeConfirmKill = () => {
    update((state) => ({
      ...state,
      showConfirmModal: false,
      processToKill: null,
      killTreeCount: null,
      killAppCount: null,
    }));
  };

  const handleConfirmKill = async () => {
    let processToKill: Process | null = null;

    let currentState: ProcessStore | undefined;
    const unsubscribe = subscribe((state) => {
      currentState = state;
    });
    unsubscribe();

    if (currentState?.processToKill && "pid" in currentState.processToKill) {
      processToKill = currentState.processToKill;
    }

    if (!processToKill?.pid) {
      return;
    }

    try {
      if (currentState?.killAppCount) {
        await killAppProcess(processToKill.pid);
      } else if (currentState?.killTreeCount) {
        await killProcessTree(processToKill.pid);
      } else {
        await killProcess(processToKill.pid);
      }
    } finally {
      update((state) => ({
        ...state,
        showConfirmModal: false,
        processToKill: null,
        killTreeCount: null,
        killAppCount: null,
      }));
    }
  };

  const confirmRestartProcess = (process: Process) => {
    update((state) => ({
      ...state,
      processToRestart: process,
      showRestartModal: true,
    }));
  };

  const closeConfirmRestart = () => {
    update((state) => ({
      ...state,
      showRestartModal: false,
      processToRestart: null,
    }));
  };

  const handleConfirmRestart = async () => {
    let processToRestart: Process | null = null;

    let currentState: ProcessStore | undefined;
    const unsubscribe = subscribe((state) => {
      currentState = state;
    });
    unsubscribe();

    if (
      currentState?.processToRestart &&
      "pid" in currentState.processToRestart
    ) {
      processToRestart = currentState.processToRestart;
    }

    if (!processToRestart?.pid) {
      return;
    }

    try {
      await restartProcess(processToRestart.pid);
    } finally {
      update((state) => ({
        ...state,
        showRestartModal: false,
        processToRestart: null,
      }));
    }
  };

  // Return all methods
  return {
    subscribe,
    set,
    update,
    setIsLoading,
    showNotice,
    getProcesses,
    killProcess,
    killAppProcess,
    restartProcess,
    toggleSuspend,
    toggleSort,
    togglePin,
    setSearchTerm,
    setIsFrozen,
    setCurrentPage,
    showProcessDetails,
    closeProcessDetails,
    confirmKillProcess,
    confirmKillTreeProcess,
    confirmKillAppProcess,
    closeConfirmKill,
    handleConfirmKill,
    confirmRestartProcess,
    closeConfirmRestart,
    handleConfirmRestart,
  };
}

export const processStore = createProcessStore();
