import { useEffect, useMemo, useState } from "react";

import { driverMode } from "../api";
import type { DeviceInfo } from "../types";
import { AgentSection } from "./settings/AgentSection";
import { DesktopBridgeSection } from "./settings/DesktopBridgeSection";
import { GroupSyncSection } from "./settings/GroupSyncSection";
import { LegacyAgentSection } from "./settings/LegacyAgentSection";
import { LocalApiSection } from "./settings/LocalApiSection";
import { StreamQualitySection } from "./settings/StreamQualitySection";
import { UpdateSection } from "./settings/UpdateSection";
import { WifiAdbSection } from "./settings/WifiAdbSection";

interface Props {
  devices: DeviceInfo[];
  deviceLabels?: ReadonlyMap<string, string>;
}

/**
 * The Settings page: eight sections, each owning its own state.
 *
 * It was 734 lines and 24 `useState` in one component, and the split was already drawn —
 * every piece of that state belonged to exactly one `<section>`, and the mount effect was
 * one independent load per section stacked into a single callback. The only value genuinely
 * shared is the driver mode, which one section reads and none writes, so it stays here.
 */
export function SettingsPanel({ devices, deviceLabels }: Props) {
  const [mode, setMode] = useState("...");

  useEffect(() => {
    driverMode()
      .then(setMode)
      .catch(() => setMode("unknown"));
  }, []);

  const connectedDevices = useMemo(
    () => devices.filter((device) => device.status !== "disconnected"),
    [devices],
  );
  const connectedUdids = useMemo(
    () => connectedDevices.map((device) => device.udid),
    [connectedDevices],
  );

  return (
    <div className="settings-page">
      <div className="settings-span-full">
        <AgentSection
          connectedDevices={connectedDevices}
          connectedUdids={connectedUdids}
          deviceLabels={deviceLabels}
        />
      </div>
      <StreamQualitySection />
      <GroupSyncSection />
      <WifiAdbSection />
      <LocalApiSection />
      <UpdateSection />
      <DesktopBridgeSection mode={mode} />
      <LegacyAgentSection />
    </div>
  );
}
