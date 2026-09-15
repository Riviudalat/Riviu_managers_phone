import type { NurtureSettings } from "./types";

export const nurtureActions = [
  { key: "like", label: "Tim", enabled: "likeEnabled", rate: "likeProb", color: "#dc6044" },
  { key: "save", label: "Lưu bài", enabled: "saveEnabled", rate: "saveProb", color: "#c38b35" },
  { key: "comment", label: "Bình luận", enabled: "commentEnabled", rate: "commentProb", color: "#4c8078" },
  { key: "follow", label: "Theo dõi", enabled: "followEnabled", rate: "followProb", color: "#647695" },
] as const;
export type NurtureRate = typeof nurtureActions[number]["rate"];

export function distributionOf(settings: NurtureSettings) {
  const actions = nurtureActions.map(action => ({ ...action, value: settings[action.enabled] === false ? 0 : Math.max(0, Math.min(100, Math.round(settings[action.rate] ?? 0))) }));
  const total = actions.reduce((sum, action) => sum + action.value, 0);
  return { actions, total, watch: Math.max(0, 100 - total) };
}

/** Upgrade only the reviewed draft. Existing saved runs keep their own selection mode. */
export function asDistribution(settings: NurtureSettings): NurtureSettings {
  const next = { ...settings, actionSelection: "exclusive" as const };
  const { actions, total } = distributionOf(next);
  if (total <= 100) return next;
  let left = 100;
  const active = actions.filter(action => action.value > 0);
  active.forEach((action, index) => {
    const value = index === active.length - 1 ? left : Math.floor(action.value * 100 / total);
    next[action.rate] = value;
    left -= value;
  });
  return next;
}

export function changeDistribution(settings: NurtureSettings, rate: NurtureRate, value: number): NurtureSettings {
  const next = asDistribution(settings);
  const action = nurtureActions.find(action => action.rate === rate)!;
  const others = distributionOf(next).actions.reduce((sum, entry) => sum + (entry.rate === rate ? 0 : entry.value), 0);
  return { ...next, [rate]: Math.max(0, Math.min(100 - others, Math.round(value) || 0)), [action.enabled]: true };
}
