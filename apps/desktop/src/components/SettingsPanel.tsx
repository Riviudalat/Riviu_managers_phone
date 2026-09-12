import { useEffect, useMemo, useState } from "react";
import { Cable, Monitor, ShieldCheck } from "lucide-react";

import { driverMode } from "../api";
import type { DeviceInfo } from "../types";
import { AgentSection } from "./settings/AgentSection";
import { DesktopBridgeSection } from "./settings/DesktopBridgeSection";
import { GroupSyncSection } from "./settings/GroupSyncSection";
import { LegacyAgentSection } from "./settings/LegacyAgentSection";
import { LocalApiSection } from "./settings/LocalApiSection";
import { GuiServiceSection } from "./settings/GuiServiceSection";
import { StreamQualitySection } from "./settings/StreamQualitySection";
import { UpdateSection } from "./settings/UpdateSection";
import { WifiAdbSection } from "./settings/WifiAdbSection";

interface Props {
  devices: DeviceInfo[];
  deviceLabels?: ReadonlyMap<string, string>;
  initialSection?: "control" | "integration" | "maintenance";
  iosRuntimeIssue?: string | null;
}

/**
 * The Settings page: eight sections, each owning its own state.
 *
 * It was 734 lines and 24 `useState` in one component, and the split was already drawn —
 * every piece of that state belonged to exactly one `<section>`, and the mount effect was
 * one independent load per section stacked into a single callback. The only value genuinely
 * shared is the driver mode, which one section reads and none writes, so it stays here.
 */
export function SettingsPanel({ devices, deviceLabels, initialSection = "control", iosRuntimeIssue }: Props) {
  const [mode, setMode] = useState("...");
  const [activeSection, setActiveSection] = useState(initialSection);

  useEffect(() => {
    setActiveSection(initialSection);
    if (initialSection === "control") return;
    const section = document.getElementById(`settings-${initialSection}`);
    section?.scrollIntoView?.({ block: "start" });
    section?.focus({ preventScroll: true });
  }, [initialSection]);

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
        <a href="#settings-control" aria-current={activeSection === "control" ? "location" : undefined} onClick={() => setActiveSection("control")}><Monitor size={16} aria-hidden="true" />Hình ảnh và điều khiển</a>
        <a href="#settings-integration" aria-current={activeSection === "integration" ? "location" : undefined} onClick={() => setActiveSection("integration")}><Cable size={16} aria-hidden="true" />Kết nối và API</a>
        <a href="#settings-maintenance" aria-current={activeSection === "maintenance" ? "location" : undefined} onClick={() => setActiveSection("maintenance")}><ShieldCheck size={16} aria-hidden="true" />Bảo trì</a>
      </nav>
      <div className="settings-sections">
        <section id="settings-control" className="settings-category" tabIndex={-1} aria-labelledby="settings-control-title">
          <h2 id="settings-control-title">Hình ảnh và điều khiển</h2>
          <p className="settings-category-description">Chất lượng stream và cách đồng bộ thao tác giữa các máy.</p>
          <StreamQualitySection />
          <GroupSyncSection />
        </section>
        <section id="settings-integration" className="settings-category" tabIndex={-1} aria-labelledby="settings-integration-title">
          <h2 id="settings-integration-title">Kết nối và API</h2>
          <p className="settings-category-description">Kết nối thiết bị, API cục bộ và công cụ tự động hóa.</p>
          <WifiAdbSection />
          <LocalApiSection />
          <GuiServiceSection />
          <DesktopBridgeSection mode={mode} />
        </section>
        <section id="settings-maintenance" className="settings-category" tabIndex={-1} aria-labelledby="settings-maintenance-title">
          <h2 id="settings-maintenance-title">Bảo trì</h2>
          <p className="settings-category-description">Kiểm tra Agent, cập nhật ứng dụng và công cụ khôi phục.</p>
          <AgentSection connectedDevices={connectedDevices} connectedUdids={connectedUdids} deviceLabels={deviceLabels} iosRuntimeIssue={iosRuntimeIssue} />
          <UpdateSection />
          <LegacyAgentSection />
        </section>
      </div>
    </div>
  );
}
