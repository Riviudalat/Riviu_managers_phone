"""Read host:devices-l from an already running loopback ADB server. No adb CLI/start."""
import json
import socket


def read_exact(stream, count):
    result = bytearray()
    while len(result) < count:
        chunk = stream.recv(count - len(result))
        if not chunk:
            raise RuntimeError("inventory connection closed")
        result.extend(chunk)
    return bytes(result)


def inventory():
    request = b"host:devices-l"
    with socket.create_connection(("127.0.0.1", 5037), timeout=5) as stream:
        stream.sendall(f"{len(request):04x}".encode() + request)
        if read_exact(stream, 4) != b"OKAY":
            raise RuntimeError("inventory refused")
        length = int(read_exact(stream, 4), 16)
        if length > 65536:
            raise RuntimeError("inventory response exceeds limit")
        lines = read_exact(stream, length).decode("utf8").splitlines()
    rows = []
    for line in lines:
        serial, state, *details = line.split()
        rows.append({"serial": serial, "state": state, "details": " ".join(details)})
    return rows


if __name__ == "__main__":
    print(json.dumps(inventory(), indent=2))
