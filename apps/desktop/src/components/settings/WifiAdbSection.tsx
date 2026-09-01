import { useState } from "react";
import { arpScan, wifiAdbConnect, type ArpEntry } from "../../api";
import { describeError } from "../../describeError";

/** Connecting a phone over Wi-Fi adb, and scanning ARP to find one. */
export function WifiAdbSection() {
  const [wifiHost, setWifiHost] = useState("");
  const [arp, setArp] = useState<ArpEntry[]>([]);
  const [arpBusy, setArpBusy] = useState(false);
  const [wifiMessage, setWifiMessage] = useState<string | null>(null);

  const connectWifi = async (host: string) => {
    const target = host.includes(":") ? host : `${host}:5555`;
    try {
      await wifiAdbConnect(target);
      setWifiMessage(`Đã kết nối ${target}. Bấm "Làm mới" ở Quản lý cửa sổ để thấy máy.`);
    } catch (error) {
      setWifiMessage(describeError(error));
    }
  };

  const scanArp = async () => {
    setArpBusy(true);
    setWifiMessage(null);
    try {
      setArp(await arpScan());
    } catch (error) {
      setWifiMessage(describeError(error));
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
      {arp.length > 0 && (
        <div className="group-tools-preview" style={{ marginTop: "0.4rem" }}>
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
      {wifiMessage && <p className="hint">{wifiMessage}</p>}
    </section>
  );
}
