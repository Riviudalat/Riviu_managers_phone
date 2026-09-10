//! A second ADB host can replace Appium's only session even while our USB lease is held.
use super::*;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn proc_address(raw: &str) -> anyhow::Result<IpAddr> {
    anyhow::ensure!(
        matches!(raw.len(), 8 | 32),
        "invalid ADB peer address length"
    );
    let mut bytes = Vec::new();
    for word in raw.as_bytes().chunks_exact(8) {
        bytes.extend(u32::from_str_radix(std::str::from_utf8(word)?, 16)?.to_le_bytes());
    }
    if bytes.len() == 4 {
        return Ok(Ipv4Addr::from(<[u8; 4]>::try_from(bytes.as_slice())?).into());
    }
    let address = Ipv6Addr::from(<[u8; 16]>::try_from(bytes.as_slice())?);
    Ok(address
        .to_ipv4_mapped()
        .map(IpAddr::V4)
        .unwrap_or(IpAddr::V6(address)))
}

fn adb_peers(raw: &str) -> anyhow::Result<Vec<IpAddr>> {
    let mut lines = raw.lines();
    let port = lines.next().context("missing ADB TCP property")?.trim();
    if matches!(port, "" | "0" | "-1") {
        return Ok(Vec::new());
    }
    let port = port.parse::<u16>().context("unreadable ADB TCP port")?;
    let mut headers = 0;
    let mut peers = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() == Some(&"sl") && fields.get(1) == Some(&"local_address") {
            headers += 1;
            continue;
        }
        anyhow::ensure!(
            fields.len() >= 4 && fields[0].ends_with(':'),
            "unreadable ADB TCP socket table"
        );
        let (_, local_port) = fields[1].rsplit_once(':').context("missing local port")?;
        let local_port = u16::from_str_radix(local_port, 16)?;
        if local_port != port || fields[3] != "01" {
            continue;
        }
        let (remote, _) = fields[2].rsplit_once(':').context("missing remote port")?;
        let address = proc_address(remote)?;
        if !address.is_loopback() && !peers.contains(&address) {
            peers.push(address);
        }
    }
    anyhow::ensure!(
        headers == 2,
        "could not read both Android TCP socket tables"
    );
    Ok(peers)
}

impl AndroidDriver {
    pub(super) async fn verify_adb_transport(&self, serial: &str) -> anyhow::Result<()> {
        let raw = self
            .adb
            .shell(
                serial,
                "getprop service.adb.tcp.port; cat /proc/net/tcp; cat /proc/net/tcp6",
            )
            .await?;
        let peers = adb_peers(&raw)?;
        let mut competing = Vec::new();
        for peer in peers {
            // A UDP connect selects our local route/address without sending a packet.
            // A TCP ADB connection from this computer is legitimate. Compare addresses,
            // never infer transport or identity from an opaque device serial.
            let bind = if peer.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            };
            let socket = std::net::UdpSocket::bind(bind)?;
            socket.connect((peer, 5555))?;
            if socket.local_addr()?.ip() != peer {
                competing.push(peer);
            }
        }
        if !competing.is_empty() {
            anyhow::bail!("Máy {serial} còn máy khác điều khiển ADB qua mạng: {}. Ngắt kết nối ở máy phụ hoặc chuyển phone đang cắm cáp sang chế độ ADB USB rồi kiểm tra lại.",competing.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const HEADER: &str = "sl local_address rem_address st tx_queue";
    #[test]
    fn usb_phone_reports_mapped_ipv6_peer_and_ignores_listener_loopback_and_closed() {
        let raw=format!("5555\n{HEADER}\n0: 5701A8C0:15B3 0100007F:C81D 01 0\n1: 5701A8C0:15B3 2B01A8C0:C81D 06 0\n{HEADER}\n2: 0000000000000000FFFF00005701A8C0:15B3 0000000000000000FFFF00002B01A8C0:C81D 01 0\n3: 00000000000000000000000000000000:15B3 00000000000000000000000000000000:0000 0A 0\n");
        assert_eq!(
            adb_peers(&raw).unwrap(),
            vec!["192.168.1.43".parse::<IpAddr>().unwrap()]
        );
    }
    #[test]
    fn unreadable_tables_do_not_claim_no_conflict_and_usb_only_needs_no_tcp_access() {
        assert!(adb_peers("5555\ncat: permission denied\n").is_err());
        assert!(adb_peers(&format!("5555\n{HEADER}\n")).is_err());
        assert!(adb_peers("0\ncat: permission denied\n").unwrap().is_empty());
        assert!(proc_address("00000000ZZZZ0000FFFF00002B01A8C0").is_err());
    }
}
