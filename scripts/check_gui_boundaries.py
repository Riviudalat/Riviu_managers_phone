"""Keep perception independent from device effects and campaign persistence."""
from pathlib import Path
import ast

root = Path(__file__).resolve().parents[1]
errors = []
for path in (root / "sidecars/gui-service/riviu_gui").glob("*.py"):
    tree = ast.parse(path.read_text(encoding="utf8"))
    for node in ast.walk(tree):
        modules = []
        if isinstance(node, ast.Import):
            modules = [n.name for n in node.names]
        elif isinstance(node, ast.ImportFrom):
            modules = [node.module or ""]
        if any(name.split(".")[0] in {"subprocess", "sqlite3", "adbutils", "uiautomator2"} for name in modules):
            errors.append(f"{path.name}: perception module imports device/persistence runtime")
for path in (root / "crates/core/src/ui_automation").glob("*.rs"):
    if path.name.endswith("tests.rs"):
        continue
    source = path.read_text(encoding="utf8")
    if ".tap(" in source or ".type_text(" in source or ".back(" in source:
        errors.append(f"{path.name}: resolver dispatches an effect")
    if "tiktok_" in source and path.name not in {"checks.rs"}:
        errors.append(f"{path.name}: shared observation depends on TikTok")
if errors:
    raise SystemExit("\n".join(errors))
print("GUI boundaries: perception has no device or campaign mutations")
