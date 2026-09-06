/**
 * Mention parsing + resolution for the Interaction feature.
 *
 * Pure and DOM-free so it is unit-testable. The operator types a free-form tag string; this
 * turns it into clean handles and, given each phone's stored @handle, works out which fleet
 * phones a tag names — those get pulled into the campaign as actors so the tagged account's
 * own phone joins the post and replies (see `ThreadCampaignRequest.mentions`/`actorUdids`).
 */

/** Operator-assigned account mapping, not a live login verification. */
export interface DeviceHandle {
  udid: string;
  handle: string;
}

export function normalizeDeviceHandle(value: string): string {
  const handle = value.trim().replace(/^@+/, "");
  if (handle && (!/^[a-zA-Z0-9_.]{1,24}$/.test(handle) || handle.endsWith("."))) {
    throw new Error("Nhập username TikTok, không nhập tên hiển thị hoặc đường link.");
  }
  return handle;
}

/**
 * Parse a free-form tag string into distinct handles, without the leading `@`.
 *
 * Accepts `@` prefixes, and separators of whitespace, comma or semicolon, so
 * `"@ann, bob;  @ann"` becomes `["ann", "bob"]`. Case is preserved for display; dedup is
 * case-insensitive so `@Ann` and `@ann` are the same tag. Order follows first appearance.
 */
export function parseMentions(text: string): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of text.split(/[\s,;]+/)) {
    const handle = raw.replace(/^@+/, "").trim();
    if (!handle) continue;
    const key = handle.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(handle);
  }
  return out;
}

/**
 * The udids of fleet phones whose stored @handle matches one of `mentions`.
 *
 * Case-insensitive, ignores devices with a blank handle, and returns each udid at most once
 * even if two mentions somehow map to it. These are the phones to add to the actor set so
 * the tagged account comes into the post itself.
 */
export function resolveMentionActors(mentions: string[], devices: DeviceHandle[]): string[] {
  const wanted = new Set(mentions.map((handle) => handle.toLowerCase()));
  const out: string[] = [];
  const added = new Set<string>();
  for (const device of devices) {
    const handle = device.handle.trim().replace(/^@+/, "").toLowerCase();
    if (!handle || !wanted.has(handle) || added.has(device.udid)) continue;
    if (devices.filter((other) => other.handle.trim().replace(/^@+/, "").toLowerCase() === handle).length !== 1) continue;
    added.add(device.udid);
    out.push(device.udid);
  }
  return out;
}

/** Union `extra` udids onto `base`, preserving order and dropping duplicates. */
export function unionActors(base: string[], extra: string[]): string[] {
  const out = [...base];
  const seen = new Set(base);
  for (const udid of extra) {
    if (!seen.has(udid)) {
      seen.add(udid);
      out.push(udid);
    }
  }
  return out;
}
