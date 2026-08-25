import { describe, expect, it } from "vitest";

import { randomAndroidId, randomMac, randomSerial } from "./randomIdentity";

/**
 * The identities a batch identity change invents, against the grammars Rust enforces.
 *
 * These three values are interpolated into `su -c` on the phone, so `adb.rs` validates every
 * one of them first — `validate_android_id`, `validate_serial_no`, `validate_mac`. A
 * generator that drifts out of those grammars does not produce a bad identity; it produces a
 * batch that is refused, per phone, after the operator pressed the button.
 *
 * The Rust half of this pin is `the_generated_identities_match_the_shapes_the_frontend_sends`
 * in `adb.rs`, which asserts the validators accept exactly the shapes described here.
 */

const RUNS = 200;

describe("the identities RootTool invents", () => {
  it("android_id is 16 hex digits, which is all `validate_android_id` accepts", () => {
    for (let i = 0; i < RUNS; i += 1) {
      expect(randomAndroidId()).toMatch(/^[0-9a-f]{16}$/);
    }
  });

  it("the serial stays inside `validate_serial_no`'s alphabet and length", () => {
    // Rust allows 1..=64 of alphanumeric plus `-` and `_`; the generator is deliberately
    // narrower — 12 uppercase, no ambiguous I/O/0/1, because a human reads these off a label.
    for (let i = 0; i < RUNS; i += 1) {
      const serial = randomSerial();
      expect(serial).toHaveLength(12);
      expect(serial).toMatch(/^[A-HJ-NP-Z2-9]{12}$/);
    }
  });

  it("the MAC is six colon-separated hex octets, which is all `ip link set` takes", () => {
    for (let i = 0; i < RUNS; i += 1) {
      expect(randomMac()).toMatch(/^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/);
    }
  });

  it("the MAC is locally administered and unicast, never a real vendor address", () => {
    // Bit 1 of the first octet set marks it locally administered; bit 0 clear marks it
    // unicast. Without both, the fleet would be inventing addresses out of a real OUI range
    // and a multicast address would simply not work.
    for (let i = 0; i < RUNS; i += 1) {
      const first = Number.parseInt(randomMac().slice(0, 2), 16);
      expect(first & 0b10).toBe(0b10);
      expect(first & 0b01).toBe(0);
    }
  });

  it("two phones never get the same identity", () => {
    // The whole point of a batch change is that each phone gets its own. A generator that
    // returned a constant would pass every shape check above.
    const ids = new Set(Array.from({ length: RUNS }, () => randomAndroidId()));
    const serials = new Set(Array.from({ length: RUNS }, () => randomSerial()));
    const macs = new Set(Array.from({ length: RUNS }, () => randomMac()));
    expect(ids.size).toBe(RUNS);
    expect(serials.size).toBe(RUNS);
    expect(macs.size).toBe(RUNS);
  });
});
