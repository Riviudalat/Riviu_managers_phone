import { useEffect, useMemo, useState } from "react";
import { Cable, Monitor, ShieldCheck } from "lucide-react";

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
      <nav className="settings-navigation" aria-label="Nhóm cài đặt">
        <a href="#settings-control"><Monitor size={16} />Hình ảnh và điều khiển</a>
        <a href="#settings-integration"><Cable size={16} />Kết nối và API</a>
        <a href="#settings-maintenance"><ShieldCheck size={16} />Bảo trì</a>
      </nav>
      <div className="settings-sections">
        <section id="settings-control" className="settings-category" aria-labelledby="settings-control-title">
          <h2 id="settings-control-title">Hình ảnh và điều khiển</h2>
          <StreamQualitySection />
          <GroupSyncSection />
        </section>
        <section id="settings-integration" className="settings-category" aria-labelledby="settings-integration-title">
          <h2 id="settings-integration-title">Kết nối và API</h2>
          <WifiAdbSection />
          <LocalApiSection />
          <DesktopBridgeSection mode={mode} />
        </section>
        <section id="settings-maintenance" className="settings-category" aria-labelledby="settings-maintenance-title">
          <h2 id="settings-maintenance-title">Bảo trì</h2>
          <AgentSection connectedDevices={connectedDevices} connectedUdids={connectedUdids} deviceLabels={deviceLabels} />
          <UpdateSection />
          <LegacyAgentSection />
        </section>
      </div>
    </div>
  );
}
