import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../../api";
import { WifiAdbSection } from "./WifiAdbSection";

vi.mock("../../api", () => ({
  wifiAdbConnect: vi.fn(async () => undefined),
  arpScan: vi.fn(async () => []),
}));

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

/**
 * The Wi-Fi adb section, reachable on its own.
 *
 * It used to be seventy lines inside a 734-line `SettingsPanel`, so the one rule it carries
 * — a host typed without a port means :5555 — could only be reached by rendering the whole
 * Settings page, its agent list, its update check and its API config first.
 */
afterEach(cleanup);

describe("WifiAdbSection", () => {
  beforeEach(() => {
    for (const fn of Object.values(mocked)) fn.mockReset();
    mocked.wifiAdbConnect.mockResolvedValue(undefined);
    mocked.arpScan.mockResolvedValue([]);
  });

  function connectTo(host: string) {
    const view = render(<WifiAdbSection />);
    const field = view.getByPlaceholderText(/192\.168/i);
    fireEvent.change(field, { target: { value: host } });
    fireEvent.click(view.getByRole("button", { name: /kết nối/i }));
    return view;
  }

  it("assumes the default adb port when the operator types only an address", async () => {
    // `adb tcpip` puts adbd on 5555 and nothing in the UI says so, so a bare IP is the
    // normal thing to type. Sending it without a port fails with a message about the host.
    connectTo("192.168.1.40");
    await waitFor(() => expect(mocked.wifiAdbConnect).toHaveBeenCalledWith("192.168.1.40:5555"));
  });

  it("leaves an explicit port alone", async () => {
    connectTo("192.168.1.40:5037");
    await waitFor(() => expect(mocked.wifiAdbConnect).toHaveBeenCalledWith("192.168.1.40:5037"));
  });

  it("reports a refused connection instead of claiming success", async () => {
    mocked.wifiAdbConnect.mockRejectedValue(new Error("connection refused"));
    const view = connectTo("192.168.1.40");
    expect(await view.findByText(/connection refused/)).toBeTruthy();
  });
});
