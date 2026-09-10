import { reconcileAssignments } from "./publishAssignments";

/** One ready machine per post; remaining source posts stay available for the next batch. */
export function allocateQuickPosts(input: {
  sourceIds: string[]; selectedIds: string[]; assignments: Record<string, string>;
  eligibleIds: string[]; readyIds: string[]; picked: string[];
}) {
  const selected = new Set(input.selectedIds);
  const candidates = input.sourceIds.filter(id => !selected.size || selected.has(id)).slice(0, 100);
  const eligible = new Set(input.eligibleIds);
  const ready = [...new Set(input.readyIds)].filter(id => eligible.has(id));
  const assignments = reconcileAssignments(candidates, input.assignments, ready);
  const picked = new Set(input.picked), used = new Set(Object.values(assignments));
  const available = ready.filter(id => !used.has(id) && (!picked.size || picked.has(id)));
  for (const id of candidates) {
    if (!assignments[id] && available.length) assignments[id] = available.shift()!;
  }
  const ids = candidates.filter(id => Boolean(assignments[id]));
  return { ids, assignments, picked: [...new Set(Object.values(assignments))], missing: [] as string[] };
}
