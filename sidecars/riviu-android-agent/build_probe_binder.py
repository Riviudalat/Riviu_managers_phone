"""Offline build of ProbeBinder only; no APK assembly, deployment, or device calls."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "build/probe/binder"


def main():
    sdk = Path(os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
               or Path(os.environ["LOCALAPPDATA"]) / "Android/Sdk")
    javac = Path(shutil.which("javac") or "javac")
    java = javac.parent / "java.exe"
    platform = sdk / "platforms/android-34/android.jar"
    d8 = sdk / "build-tools/34.0.0/lib/d8.jar"
    source = ROOT / "ProbeBinder.java"
    carrier = ROOT / "app/src/main/java/com/riviu/agent/BootstrapProviderCarrier.java"
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    carrier_hash = hashlib.sha256(carrier.read_bytes()).hexdigest()
    identity_hash = hashlib.sha256(source.read_bytes() + carrier.read_bytes()).hexdigest()
    # A fresh identity directory excludes anonymous classes left by the old bindService probe.
    classes = OUT / identity_hash / "classes"
    dex = OUT / identity_hash / "dex"
    classes.mkdir(parents=True, exist_ok=True)
    dex.mkdir(parents=True, exist_ok=True)
    ledger = []

    def run(*command):
        argv = [str(x) for x in command]
        result = subprocess.run(argv, capture_output=True, text=True)
        ledger.append(dict(command=argv, stdout=result.stdout, stderr=result.stderr, exit=result.returncode))
        (OUT / "VERIFICATION.txt").write_text(json.dumps(ledger, indent=2), encoding="utf-8")
        print(result.stdout, end="")
        print(result.stderr, end="")
        if result.returncode:
            raise SystemExit(result.returncode)

    run(javac, "-source", "8", "-target", "8", "-nowarn", "-encoding", "UTF-8",
        "-classpath", platform, "-d", classes, source, carrier)
    run(java, "-cp", d8, "com.android.tools.r8.D8", "--lib", platform, "--min-api", "26",
        "--output", dex, *sorted(classes.glob("ProbeBinder*.class")),
        *sorted((classes / "com/riviu/agent").glob("ProbeBinder*.class")),
        *sorted((classes / "com/riviu/agent").glob("BootstrapProviderCarrier*.class")))
    artifact = OUT / ("probe-binder-provider-v3-" + identity_hash[:16] + ".jar")
    with zipfile.ZipFile(artifact, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.write(dex / "classes.dex", "classes.dex")
    with zipfile.ZipFile(artifact) as archive:
        assert archive.namelist() == ["classes.dex"]
        assert archive.read("classes.dex")[:4] == b"dex\n"
    def sha(path):
        return hashlib.sha256(path.read_bytes()).hexdigest()
    metadata = dict(artifact=str(artifact), sha256=sha(artifact), bytes=artifact.stat().st_size,
                    diagnosticIdentity="binder-provider-v3",
                    carrierSourceSha256=carrier_hash, buildIdentitySha256=identity_hash,
                    sourceSha256=sha(source), androidJarSha256=sha(platform), d8Sha256=sha(d8),
                    command="CLASSPATH=PINNED_APK:PROBE_JAR app_process /system/bin com.riviu.agent.ProbeBinder CHECKED_UID CERT_SHA256 SERVICE_INSTANCE OWNER_GENERATION </dev/null",
                    qualification="offline compile and DEX packaging only; device execution is coordinator-owned")
    (OUT / "provenance.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    print(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
