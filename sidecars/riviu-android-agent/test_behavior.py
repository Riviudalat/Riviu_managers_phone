"""Offline Java behavior runner. Candidate build first compiles all actual Android sources."""
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
JAVA = Path(os.environ["JAVA_HOME"]) / "bin"
SDK = Path(os.environ.get("ANDROID_HOME") or os.environ["ANDROID_SDK_ROOT"])
CLASSES = ROOT / "build/candidate/classes"
OUT = ROOT / "build/behavior"


def run(command):
    result = subprocess.run([str(part) for part in command], cwd=ROOT)
    if result.returncode:
        raise SystemExit(result.returncode)


if __name__ == "__main__":
    if not (CLASSES / "com/riviu/agent/ClipboardStore.class").is_file():
        raise SystemExit("Run build.ps1 -Candidate first; tests require actual SDK-compiled sources")
    OUT.mkdir(parents=True, exist_ok=True)
    run([JAVA / "javac.exe", "-source", "8", "-target", "8", "-nowarn", "-encoding", "UTF-8",
         "-classpath", os.pathsep.join([str(CLASSES), str(SDK / "platforms/android-34/android.jar")]),
         "-d", OUT, ROOT / "tests/com/riviu/agent/BehaviorTests.java"])
    run([JAVA / "java.exe", "-classpath", os.pathsep.join([str(OUT), str(CLASSES)]),
         "com.riviu.agent.BehaviorTests"])
