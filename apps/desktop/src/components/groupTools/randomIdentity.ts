/**
 * The identities `RootTool` invents for a batch identity change.
 *
 * Apart from the component because they are pure and because the Rust side validates
 * every one of them before it reaches `su -c` — `validate_android_id`, `validate_serial_no`
 * and `validate_mac`. A generator that drifts out of those grammars is a batch that fails
 * on the phone rather than in the form, so the two halves are pinned by a test.
 */
/** Random bytes, from the CSPRNG where present, else `Math.random` on old webviews. */
export function randomBytes(n: number): Uint8Array {
  const bytes = new Uint8Array(n);
  try {
    crypto.getRandomValues(bytes);
  } catch {
    for (let i = 0; i < n; i += 1) bytes[i] = Math.floor(Math.random() * 256);
  }
  return bytes;
}

/** 16 hex chars — the shape of a `Settings.Secure` android_id. */
export function randomAndroidId(): string {
  return Array.from(randomBytes(8), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** A plausible device serial: 12 uppercase alphanumerics (no ambiguous I/O/0/1). */
export function randomSerial(): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  return Array.from(randomBytes(12), (b) => alphabet[b % alphabet.length]).join("");
}

/** A locally-administered unicast MAC (first octet: bit 1 set, bit 0 clear). */
export function randomMac(): string {
  const bytes = randomBytes(6);
  bytes[0] = (bytes[0] & 0xfe) | 0x02;
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(":");
}
