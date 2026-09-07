/** Bundle identity owns its device choice; changing a selection never shifts its siblings. */
export function reconcileAssignments(
  ids: string[],
  assignments: Record<string, string>,
  eligible: string[],
) {
  const allowed = new Set(eligible),
    used = new Set<string>();
  return Object.fromEntries(
    ids.flatMap((id) => {
      const udid = assignments[id];
      if (!udid || !allowed.has(udid) || used.has(udid)) return [];
      used.add(udid);
      return [[id, udid]];
    }),
  );
}

export function assignDevice(
  assignments: Record<string, string>,
  bundleId: string,
  udid: string,
) {
  const next = { ...assignments },
    previous = next[bundleId];
  const other = Object.keys(next).find(
    (id) => id !== bundleId && next[id] === udid,
  );
  if (other) {
    if (previous) next[other] = previous;
    else delete next[other];
  }
  next[bundleId] = udid;
  return next;
}

export function fillAssignments(
  ids: string[],
  assignments: Record<string, string>,
  eligible: string[],
  startIndex = 0,
) {
  const next = reconcileAssignments(ids, assignments, eligible);
  const available = eligible.slice(startIndex).filter(
    (udid) => !Object.values(next).includes(udid),
  );
  const missing = ids.filter((id) => !next[id]);
  if (missing.length > available.length) return null;
  missing.forEach((id, i) => {
    next[id] = available[i];
  });
  return next;
}
