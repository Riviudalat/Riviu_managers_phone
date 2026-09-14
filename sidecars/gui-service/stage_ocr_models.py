"""Build-time only: fetch the upstream-pinned local OCR models, then verify each byte."""

import argparse
import hashlib
import json
import pathlib
import urllib.request

SOURCE = pathlib.Path(__file__).resolve().parent
MANIFEST = SOURCE / "riviu_gui/ocr-models.json"


def stage(output: pathlib.Path) -> None:
    manifest = json.loads(MANIFEST.read_text(encoding="utf8"))
    output.mkdir(parents=True, exist_ok=True)
    for entry in manifest["files"]:
        destination = output / entry["path"]
        if destination.is_file():
            with destination.open("rb") as stream:
                if hashlib.file_digest(stream, "sha256").hexdigest() == entry["sha256"]:
                    continue
        temporary = destination.with_suffix(".download")
        try:
            with urllib.request.urlopen(entry["url"], timeout=120) as response, temporary.open("wb") as stream:
                size = 0
                while chunk := response.read(1024 * 1024):
                    size += len(chunk)
                    if size > 64 * 1024 * 1024:
                        raise ValueError("OCR model exceeds build download budget")
                    stream.write(chunk)
            with temporary.open("rb") as stream:
                if hashlib.file_digest(stream, "sha256").hexdigest() != entry["sha256"]:
                    raise ValueError(f"OCR model hash mismatch: {entry['path']}")
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)
    (output / "ocr-models.json").write_bytes(MANIFEST.read_bytes())
    print(json.dumps({"ok": True, "models": len(manifest["files"]), "output": str(output.resolve())}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, default=SOURCE.parents[1] / "target/ocr-models")
    stage(parser.parse_args().output)
