import { useEffect, useSyncExternalStore } from "react";
import { listDeviceWorkStates, listDeviceMetas, setWallpaperBytes, setWallpaper } from "./api";
import { describeError } from "./describeError";
import { numberWallpaperPng } from "./numberWallpaper";
import type { DeviceInfo } from "./types";

const KEY = "riviu.numberWallpaper.v1";
const TEMPLATE = "r-white-1";
type Entry = { enabled: boolean; signature?: string; state?: "pending" | "applied" | "needsReview"; error?: string };
type State = Record<string, Entry>;
function load(): State {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
    return Object.fromEntries(Object.entries(raw).filter((row): row is [string, Entry] => {
      const entry = row[1];
      return Boolean(entry && typeof entry === "object" && typeof entry.enabled === "boolean"
        && (entry.signature === undefined || typeof entry.signature === "string")
        && (entry.error === undefined || typeof entry.error === "string")
        && (entry.state === undefined || ["pending", "applied", "needsReview"].includes(entry.state)));
    }));
  } catch { return {}; }
}
let state = load();
const listeners = new Set<() => void>();
const inFlight = new Set<string>();
function commit(next: State) {
  // Persist intent before dispatch. A lost ACK or restart never silently replays wallpaper input.
  localStorage.setItem(KEY, JSON.stringify(next));
  state = next;
  listeners.forEach(listener => listener());
}
function importHolds(udid: string): boolean {
  try {
    const raw = localStorage.getItem("riviu.pendingMetadataImport.v1");
    if (!raw) return false;
    const pending = JSON.parse(raw);
    if (!Array.isArray(pending?.input?.devices)) return true;
    return pending.input.devices.some((device: { udid?: string }) => device.udid === udid);
  } catch { return true; }
}
export function setWallpaperSync(udids: string[], enabled: boolean) {
  if (enabled && udids.some(importHolds)) throw new Error("Lượt nhập danh sách chưa xác nhận; đọc lại kết quả trước khi bật hình nền.");
  const next = { ...state };
  for (const udid of udids) next[udid] = { ...next[udid], enabled };
  commit(next);
}
export function useWallpaperSyncState() {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, () => state);
}
/** Explicit retry is the only route after an uncertain/failed command. */
export function retryWallpaperSync(udids: string[]) {
  const next = { ...state };
  for (const udid of udids) if (next[udid] && ["pending", "needsReview"].includes(next[udid].state ?? "") && !inFlight.has(udid)) next[udid] = { enabled: next[udid].enabled };
  commit(next);
}
/** Metadata import must not silently activate a device effect, including after lost ACK. */
export function pauseImportedWallpapers(udids: string[]) {
  const next = { ...state };
  for (const udid of udids) {
    if (inFlight.has(udid)) throw new Error("Đang đặt hình nền; chờ hoàn tất trước khi nhập danh sách máy.");
    next[udid] = { ...next[udid], enabled: false };
  }
  commit(next);
}
async function applyWallpaper(udid: string, signature: string, prepare: () => Promise<() => Promise<void>>, active: () => boolean, custom = false, force = false) {
  const entry = state[udid];
  if (importHolds(udid)) throw new Error("Lượt nhập danh sách chưa xác nhận; chưa đặt hình nền.");
  if (inFlight.has(udid) || entry?.state === "pending" || entry?.state === "needsReview") {
    throw new Error("Lần đặt hình nền trước chưa kết thúc hoặc chưa rõ kết quả; kiểm tra trước khi thử lại.");
  }
  if (!custom && !force && entry?.signature === signature && entry.state === "applied") return;
  inFlight.add(udid);
  let dispatched = false;
  try {
    const send = await prepare();
    if (!active()) return;
    commit({ ...state, [udid]: { enabled: custom ? false : state[udid]?.enabled ?? false, signature, state: "pending" } });
    dispatched = true;
    await send();
    commit({ ...state, [udid]: { enabled: state[udid]?.enabled ?? false, signature, state: "applied" } });
  } catch (error) {
    if (dispatched) commit({ ...state, [udid]: { enabled: state[udid]?.enabled ?? false, signature, state: "needsReview", error: describeError(error) } });
    throw error;
  } finally { inFlight.delete(udid); }
}
export function applyNumberWallpaper(udid: string, number: number, active: () => boolean = () => true, force = true) {
  if (!Number.isInteger(number) || number < 1) return Promise.reject(new Error("Máy chưa có số đã lưu."));
  return applyWallpaper(udid, `${TEMPLATE}:${number}`, async () => {
    const png = await numberWallpaperPng(String(number));
    return () => setWallpaperBytes(udid, Array.from(png));
  }, active, false, force);
}
export function applyCustomWallpaper(udid: string, path: string) {
  return applyWallpaper(udid, "custom", async () => () => setWallpaper(udid, path), () => true, true);
}
export async function syncNumberWallpapers(devices: DeviceInfo[], active: () => boolean = () => true) {
  const wanted = devices.filter(device => device.platform === "android" && device.status !== "disconnected" && state[device.udid]?.enabled && !inFlight.has(device.udid));
  if (!wanted.length || !active()) return;
  const [work, metas] = await Promise.all([listDeviceWorkStates(), listDeviceMetas()]);
  for (const device of wanted) {
    if (!active()) return;
    const udid = device.udid;
    const held = work.find(row => row.udid === udid);
    const number = metas.find(row => row.udid === udid)?.number;
    const entry = state[udid];
    if (!held || held.currentOwner || number == null || !entry?.enabled || inFlight.has(udid)) continue;
    if (entry.state === "pending" || entry.state === "needsReview") continue;
    try { await applyNumberWallpaper(udid, number, () => active() && Boolean(state[udid]?.enabled), false); }
    catch { /* Command uncertainty is retained by the shared owner; no automatic replay. */ }
  }
}
/** Runs only while the desktop is open; same existing lease-protected wallpaper command. */
export function useNumberWallpaperSync(devices: DeviceInfo[]) {
  const settings = useWallpaperSyncState();
  useEffect(() => {
    if (!Object.values(settings).some(entry => entry.enabled)) return;
    let active = true;
    let running = false;
    const tick = async () => {
      if (running || !active) return;
      running = true;
      try { await syncNumberWallpapers(devices, () => active); }
      catch { /* A metadata/ownership read failure sends nothing; next observation can retry. */ }
      finally { running = false; }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 15_000);
    return () => { active = false; window.clearInterval(timer); };
  }, [devices, settings]);
}
