import { useCallback, useEffect, useRef, useState } from "react";
import { getDeviceMeta, saveDeviceHandle } from "./api";
import { describeError } from "./describeError";
import { normalizeDeviceHandle } from "./interactionMentions";

type AccountDraft = { saved: string; draft: string; edited: boolean; busy: boolean; error?: string };
const empty = (): AccountDraft => ({ saved: "", draft: "", edited: false, busy: false });

/** One authoritative saved value per UDID; late reads cannot overwrite newer saves/reloads. */
export function useDeviceHandles(udids: string[]) {
  const [accounts, setAccounts] = useState<Record<string, AccountDraft>>({});
  const current = useRef(accounts);
  const tickets = useRef<Record<string, number>>({});
  const writes = useRef(new Set<string>());
  const mounted = useRef(true);
  const scopeKey = JSON.stringify(udids);
  const update = useCallback((udid: string, change: (value: AccountDraft) => AccountDraft) => {
    if (!mounted.current) return;
    const next = { ...current.current, [udid]: change(current.current[udid] ?? empty()) };
    current.current = next;
    setAccounts(next);
  }, []);
  const nextTicket = (udid: string) => (tickets.current[udid] = (tickets.current[udid] ?? 0) + 1);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  const load = useCallback(async (udid: string, discard: boolean) => {
    const ticket = nextTicket(udid);
    update(udid, (value) => ({ ...value, busy: true, error: undefined }));
    try {
      const meta = await getDeviceMeta(udid);
      if (tickets.current[udid] !== ticket) return;
      const saved = meta.handle ?? "";
      update(udid, (value) => ({ saved, draft: discard || !value.edited ? saved : value.draft,
        edited: !discard && value.edited, busy: false }));
    } catch (error) {
      if (tickets.current[udid] === ticket) update(udid, (value) => ({ ...value, busy: false, error: `Chưa đọc được tài khoản: ${describeError(error)}` }));
    }
  }, [update]);

  useEffect(() => {
    const scope = JSON.parse(scopeKey) as string[];
    const activeWrites = writes.current;
    for (const udid of scope) if (!activeWrites.has(udid)) void load(udid, false);
    return () => { for (const udid of scope) if (!activeWrites.has(udid)) nextTicket(udid); };
  }, [scopeKey, load]);

  const change = useCallback((udid: string, draft: string) => {
    update(udid, (value) => ({ ...value, draft, edited: true }));
  }, [update]);

  const persist = useCallback(async (udid: string, raw: string) => {
    if (writes.current.has(udid)) return;
    writes.current.add(udid);
    const ticket = nextTicket(udid);
    update(udid, (value) => ({ ...value, busy: true }));
    try {
      const handle = normalizeDeviceHandle(raw);
      const duplicate = Object.entries(current.current).some(([other, value]) => other !== udid
        && value.draft.trim().replace(/^@+/, "").toLowerCase() === handle.toLowerCase());
      if (handle && duplicate) throw new Error("Nick này đang gán cho máy khác; kiểm tra lại tài khoản trước khi lưu.");
      const expected = current.current[udid]?.saved ?? (await getDeviceMeta(udid)).handle ?? "";
      const saved = await saveDeviceHandle(udid, expected, handle);
      if (tickets.current[udid] !== ticket) return;
      update(udid, () => ({ saved, draft: saved, edited: false, busy: false }));
    } catch (error) {
      if (tickets.current[udid] === ticket) update(udid, (value) => ({ ...value, busy: false, error: `Chưa lưu nick: ${describeError(error)}` }));
    } finally { writes.current.delete(udid); }
  }, [update]);

  const reload = useCallback(async (udid: string) => {
    if (!writes.current.has(udid)) await load(udid, true);
  }, [load]);

  return {
    handles: Object.fromEntries(Object.entries(accounts).map(([id, value]) => [id, value.draft])),
    savedHandles: Object.fromEntries(Object.entries(accounts).map(([id, value]) => [id, value.saved])),
    handleErrors: Object.fromEntries(Object.entries(accounts).filter(([, value]) => value.error).map(([id, value]) => [id, value.error!])),
    savingHandles: Object.fromEntries(Object.entries(accounts).map(([id, value]) => [id, value.busy])),
    change, persist, reload,
  };
}
