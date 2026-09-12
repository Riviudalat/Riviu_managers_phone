import { reconcileAssignments } from "./publishAssignments";

const BATCH_CAP = 100;

/**
 * One ready machine per post. Fills every ready-in-scope machine from the source.
 * A prior partial selection does not trap the next click: when every currently selected
 * post already has a machine and free ready phones remain, candidates expand to the
 * rest of the source (up to BATCH_CAP). `picked` is ignored for capacity.
 */
export function allocateQuickPosts(input: {
  sourceIds: string[]; selectedIds: string[]; assignments: Record<string, string>;
  eligibleIds: string[]; readyIds: string[]; picked: string[];
}) {
  const selected = new Set(input.selectedIds);
  const eligible = new Set(input.eligibleIds);
  const ready = [...new Set(input.readyIds)].filter(id => eligible.has(id));
  const source = input.sourceIds.slice(0, BATCH_CAP);
  const restricted = selected.size ? source.filter(id => selected.has(id)) : source;
  const probe = reconcileAssignments(restricted, input.assignments, ready);
  const probeUsed = new Set(Object.values(probe));
  const freeMachines = ready.filter(id => !probeUsed.has(id));
  const selectedAllMapped = !selected.size || restricted.every(id => Boolean(probe[id]));
  const unusedSource = source.some(id => !restricted.includes(id));
  const expand = selectedAllMapped && freeMachines.length > 0 && unusedSource;
  const candidates = !selected.size || expand ? source : restricted;
  const assignments = reconcileAssignments(candidates, input.assignments, ready);
  const used = new Set(Object.values(assignments));
  const available = ready.filter(id => !used.has(id));
  for (const id of candidates) {
    if (!assignments[id] && available.length) assignments[id] = available.shift()!;
  }
  const ids = candidates.filter(id => Boolean(assignments[id]));
  return { ids, assignments, picked: [...new Set(Object.values(assignments))], missing: [] as string[] };
}
