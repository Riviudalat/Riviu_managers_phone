import type { GoogleSheetTarget } from "./googleSheetUrl";

export type CheckedSheet = { account: string; target: GoogleSheetTarget; ready: boolean; message: string };

let checkedSheet: CheckedSheet | null = null;
let invalidated = false;

export function getSheetVerificationSession() { return checkedSheet; }
export function isSheetVerificationInvalidated() { return invalidated; }
export function setSheetVerificationSession(value: CheckedSheet) { checkedSheet = value; invalidated = false; }
export function invalidateSheetVerificationSession() { checkedSheet = null; invalidated = true; }
export function clearSheetVerificationSession() { checkedSheet = null; invalidated = false; }
