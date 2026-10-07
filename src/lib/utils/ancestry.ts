import type { Process } from "$lib/types";

/** One breadcrumb segment of a process ancestry chain; the last one is
 * the subject. */
export interface ChainSegment {
  pid: number;
  name: string;
  /** Command line shown on hover; null when the snapshot lacks it. */
  command: string | null;
  isSubject: boolean;
}

/** How far the ppid walk may go before the chain is cut off. */
const MAX_CHAIN_DEPTH = 32;

export interface AncestryChain {
  segments: ChainSegment[];
  /** A walk that ended because a parent is missing from the snapshot. */
  broken: boolean;
  /** The DIRECT parent is missing — nothing vouches for the process. */
  orphaned: boolean;
}

/**
 * Walks the ppid chain from a process up through the snapshot, oldest
 * ancestor first, with the subject process as the final segment. Cycles
 * and self-references stop the walk; a parent that is not in the snapshot
 * marks the chain broken ("parent has exited or is not visible").
 *
 * Shared by the ports panel's deep-dive panel and the process details
 * modal ("why is this running?" is a process question, not a ports
 * question).
 */
export function buildAncestryChain(
  process: Process,
  processes: Process[],
): AncestryChain {
  const byPid = new Map(processes.map((entry) => [entry.pid, entry]));
  const segments: ChainSegment[] = [
    {
      pid: process.pid,
      name: process.name,
      command: process.command || null,
      isSubject: true,
    },
  ];
  const visited = new Set<number>([process.pid]);
  let broken = false;
  let current = process;
  for (let depth = 0; depth < MAX_CHAIN_DEPTH; depth++) {
    const ppid = current.ppid;
    if (!ppid || ppid === current.pid || visited.has(ppid)) break;
    const parent = byPid.get(ppid);
    if (!parent) {
      broken = true;
      break;
    }
    visited.add(ppid);
    segments.unshift({
      pid: parent.pid,
      name: parent.name,
      command: parent.command || null,
      isSubject: false,
    });
    current = parent;
  }
  // A missing DIRECT parent is the interesting case (nothing vouches for
  // the process); a missing grandparent-and-beyond is the Windows norm —
  // short-lived launchers (userinit, smss) exit right after spawning,
  // so nearly every GUI/service chain breaks somewhere up top.
  return { segments, broken, orphaned: broken && segments.length === 1 };
}
