/** Small helpers shared by the group tools and the popup that hosts them. */

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    // A webview old enough to lack randomUUID still needs a unique-enough id.
    return `qp-${Date.now()}-${Math.round(Math.random() * 1e9)}`;
  }
}
