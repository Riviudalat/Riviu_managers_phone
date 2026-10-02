"""Build standalone read-only app_process diagnostic DEX using installed tools only."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "build/probe"
SDK = Path(os.environ.get("ANDROID_HOME") or os.environ["ANDROID_SDK_ROOT"])
JAVA = Path(os.environ["JAVA_HOME"]) / "bin"
PLATFORM = SDK / "platforms/android-34/android.jar"
TOOLS = SDK / "build-tools/34.0.0"


def run(*command: str | Path) -> None:
    result = subprocess.run([str(value) for value in command], cwd=OUT)
    if result.returncode:
        raise SystemExit(result.returncode)


if __name__ == "__main__":
    classes = OUT / "classes"
    dex = OUT / "dex"
    classes.mkdir(parents=True, exist_ok=True)
    dex.mkdir(parents=True, exist_ok=True)
    source = ROOT / "ProbeBootstrap.java"
    run(JAVA / "javac.exe", "-parameters", "-source", "8", "-target", "8", "-nowarn",
        "-encoding", "UTF-8", "-classpath", PLATFORM, "-d", classes, source)
    run(TOOLS / "d8.bat", "--lib", PLATFORM, "--min-api", "26", "--output", dex,
        *sorted(classes.rglob("*.class")))
    artifact = OUT / "probe-bootstrap.jar"
    run(JAVA / "jar.exe", "cf", artifact, "-C", dex, "classes.dex")
    digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
    metadata = {
        "artifact": artifact.name, "bytes": artifact.stat().st_size, "sha256": digest,
        "sourceSha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "androidJarSha256": hashlib.sha256(PLATFORM.read_bytes()).hexdigest(),
        "purpose": "read-only diagnostic invalid_action or nonsecret stdin-only; no APK changes",
    }
    (OUT / "provenance.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    print(artifact)
    print(f"bytes={metadata['bytes']} sha256={digest}")
