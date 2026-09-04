import { useState } from "react";
import { arpScan, wifiAdbConnect, type ArpEntry } from "../../api";
import { describeError } from "../../describeError";
import { EmptyState, LoadingState, StatusNotice, type NoticeTone } from "../States";

/** Connecting a phone over Wi-Fi adb, and scanning ARP to find one. */
export function WifiAdbSection() {
  const [wifiHost, setWifiHost] = useState("");
  const [arp, setArp] = useState<ArpEntry[]>([]);
  const [arpBusy, setArpBusy] = useState(false);
  const [arpScanned, setArpScanned] = useState(false);
  const [wifiMessage, setWifiMessage] = useState<{ tone: NoticeTone; text: string } | null>(null);

  const connectWifi = async (host: string) => {
    const target = host.includes(":") ? host : `${host}:5555`;
    try {
      await wifiAdbConnect(target);
      setWifiMessage({ tone: "success", text: `Đã kết nối ${target}. Làm mới trang Thiết bị để thấy máy.` });
    } catch (error) {
      setWifiMessage({ tone: "error", text: describeError(error) });
    }
  };

  const scanArp = async () => {
    setArpBusy(true);
    setWifiMessage(null);
    try {
      setArp(await arpScan());
      setArpScanned(true);
    } catch (error) {
      setArpScanned(false);
      setWifiMessage({ tone: "error", text: describeError(error) });
    } finally {
      setArpBusy(false);
    }
  };
  return (
    <section className="settings-section">
      <h3>Kết nối Android qua Wi-Fi</h3>
      <p className="hint">
        Kết nối này thay cáp cho lệnh Android và sẽ mất khi điện thoại hoặc mạng đổi địa chỉ.
      </p>
      <details className="settings-details" aria-label="Điều kiện kết nối Wi-Fi">
        <summary>Điều kiện kết nối Wi-Fi</summary>
        <p className="hint">
          Bật từ menu của máy đang cắm USB hoặc nhập địa chỉ bên dưới; điện thoại và PC phải cùng mạng LAN.
        </p>
      </details>
      <div className="row">
        <label style={{ flex: 1 }}>
          Host (ip hoặc ip:cổng)
          <input
            type="text"
            placeholder="192.168.1.42 hoặc 192.168.1.42:5555"
            value={wifiHost}
            onChange={(event) => setWifiHost(event.target.value)}
          />
        </label>
        <button
          type="button"
          className="ghost"
          disabled={!wifiHost.trim()}
          onClick={() => void connectWifi(wifiHost.trim())}
        >
          Kết nối
        </button>
        <button type="button" className="ghost" disabled={arpBusy} onClick={() => void scanArp()}>
          {arpBusy ? "Đang quét…" : "Quét mạng (ARP)"}
        </button>
      </div>
      {arpBusy && <LoadingState label="Đang quét mạng…" />}
      {!arpBusy && arpScanned && arp.length === 0 && (
        <EmptyState compact title="Không thấy thiết bị trong bảng mạng" hint="Kiểm tra điện thoại và máy tính đang ở cùng mạng rồi quét lại." />
      )}
      {arp.length > 0 && (
        <div className="group-tools-preview">
          {arp.map((entry) => (
            <div className="row-item" key={entry.ip}>
              <span className="who mono">{entry.ip}</span>
              <span className="what mono">{entry.mac}</span>
              <span className="grow" />
              <button type="button" className="ghost" onClick={() => void connectWifi(entry.ip)}>
                Kết nối
              </button>
            </div>
          ))}
        </div>
      )}
      {wifiMessage && <StatusNotice tone={wifiMessage.tone}>{wifiMessage.text}</StatusNotice>}
    </section>
  );
}
