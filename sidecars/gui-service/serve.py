"""Boot from an inherited pipe; stdout contains only the loopback handshake."""

import json
import socket
import sys
import threading

import uvicorn

from riviu_gui import PROTOCOL_VERSION, VERSION
from riviu_gui.app import create_app
from riviu_gui.provider import ProviderConfig


def main():
    bootstrap = json.loads(sys.stdin.readline())
    config = ProviderConfig(**bootstrap.get("provider", {}))
    app = create_app(bootstrap["token"], config)
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    server = uvicorn.Server(uvicorn.Config(app, log_level="warning", access_log=False, workers=1))

    def parent_lifetime():
        sys.stdin.read()
        server.should_exit = True

    threading.Thread(target=parent_lifetime, daemon=True).start()
    print(
        json.dumps(
            {"port": sock.getsockname()[1], "protocolVersion": PROTOCOL_VERSION, "serviceVersion": VERSION}
        ),
        flush=True,
    )
    try:
        server.run(sockets=[sock])
    finally:
        sock.close()


if __name__ == "__main__":
    main()
