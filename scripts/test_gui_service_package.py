"""Verify the staged Python service without using a development interpreter at runtime."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request


def verify(runtime: Path):
    exe = runtime / ("riviu-gui-service.exe" if os.name == "nt" else "riviu-gui-service")
    token = "package-check-" + "f" * 48
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith(("PYTHON", "RIVIU", "VIRTUAL_ENV"))}
    env["PATH"] = os.environ.get("SystemRoot", "C:\\Windows") + "\\System32" if os.name == "nt" else "/usr/bin:/bin"
    child = subprocess.Popen([str(exe.resolve())], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, encoding="utf8",
                             creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    try:
        child.stdin.write(json.dumps({"token": token, "provider": {}}) + "\n")
        child.stdin.flush()
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(1) as reader:
            future = reader.submit(child.stdout.readline)
            try:
                hello = json.loads(future.result(timeout=15))
            except BaseException:
                child.terminate()
                raise
        assert hello["protocolVersion"] == 1
        url = f"http://127.0.0.1:{hello['port']}/health/ready"
        for _ in range(50):
            try:
                request = urllib.request.Request(url, headers={"Authorization": "Bearer " + token})
                with urllib.request.urlopen(request, timeout=1) as response:
                    ready = json.load(response)
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise AssertionError("packaged service readiness timeout")
        assert ready["ready"] and not ready["providerReady"] and ready["protocolVersion"] == 1
        child.stdin.close()
        child.wait(timeout=10)
        assert child.returncode == 0
        return {"ready": True, "protocolVersion": 1, "parentExitStopsService": True}
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=5)


if __name__ == "__main__":
    import sys
    print(json.dumps(verify(Path(sys.argv[1]))))
