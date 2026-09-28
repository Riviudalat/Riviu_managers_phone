import { parseGoogleSheetUrl, type GoogleSheetTarget } from "./googleSheetUrl";
import type { GoogleSheetVerification, GoogleSheetsStatus } from "../../types";

export type CheckedSheet = {
  account: string; target: GoogleSheetTarget; ready: boolean; message: string;
  verifiedAt?: number; expiresAt?: number;
};

// Process memory only: a new app session must obtain fresh remote evidence.
const TTL = 5 * 60_000;
let checkedSheet: CheckedSheet | null = null;
// Eligibility is not fresh remote proof; final preflight still checks rights.
let preflightBinding: string | null = null;
let invalidated = false;
let generation = 0;
const pending = new Map<string, Promise<GoogleSheetVerification>>();
const proofs = new Map<string, GoogleSheetVerification>();

export const sheetAccountKey = (s: GoogleSheetsStatus) => JSON.stringify([
  s.clientId, s.accountId, s.connected, s.active, s.writerId, s.hasSheetsScope,
  s.reportingEpoch ?? null, s.authorizationGeneration ?? 0,
  s.active ? parseGoogleSheetUrl(s.sheetUrl || "")?.url ?? s.sheetUrl : null,
]);
export const sheetVerificationKey = (account: string, target: GoogleSheetTarget) =>
  JSON.stringify([account, target.spreadsheetId, target.sheetId]);

export function getSheetPreflightBinding() { return preflightBinding; }

export function getSheetVerificationSession() {
  if (checkedSheet && (!checkedSheet.expiresAt || checkedSheet.expiresAt <= Date.now())) checkedSheet = null;
  return checkedSheet;
}
export function isSheetVerificationInvalidated() { return invalidated; }
export function setSheetVerificationSession(value: CheckedSheet) {
  const verifiedAt = value.verifiedAt ?? Date.now();
  checkedSheet = { ...value, verifiedAt, expiresAt: Math.min(value.expiresAt ?? verifiedAt + TTL, verifiedAt + TTL) };
  preflightBinding = value.ready ? sheetVerificationKey(value.account, value.target) : null;
  invalidated = false;
}
export function invalidateSheetVerificationSession() {
  checkedSheet = null; preflightBinding = null; invalidated = true; generation += 1; proofs.clear();
  // Retain reads until settlement to avoid duplicate same-key requests.
}
export function clearSheetVerificationSession() { invalidateSheetVerificationSession(); invalidated = false; }

export function verifySheetSingleFlight(key: string, read: () => Promise<GoogleSheetVerification>): Promise<GoogleSheetVerification> {
  const cached = proofs.get(key);
  if (cached && cached.expiresAt > Date.now()) return Promise.resolve(cached);
  proofs.delete(key);
  const existing = pending.get(key);
  if (existing) return existing;
  const ticket = generation;
  const flight = read().then(value => {
    if (ticket !== generation) throw Error("Kết nối đã thay đổi; đang chờ xác minh mới.");
    if (!Number.isFinite(value.verifiedAt) || !Number.isFinite(value.expiresAt)
      || value.expiresAt <= Date.now() || value.expiresAt > value.verifiedAt + TTL) {
      throw Error("Thời hạn xác minh Google Sheet không hợp lệ.");
    }
    if (proofs.size >= 16) proofs.clear();
    proofs.set(key, value);
    return value;
  }).finally(() => { if (pending.get(key) === flight) pending.delete(key); });
  pending.set(key, flight);
  return flight;
}
