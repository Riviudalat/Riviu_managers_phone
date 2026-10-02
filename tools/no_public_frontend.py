"""Launch only Vite for this worktree; does not start Tauri or access devices."""
from pathlib import Path
import os
import subprocess
import json

root=Path('.').resolve()
modules=root/'apps/desktop/node_modules'
shared=root.parents[2]/'apps/desktop/node_modules'
if not shared.is_dir():raise SystemExit('Existing frontend dependencies unavailable; no install')
if not modules.exists():
    subprocess.run(['cmd.exe','/c','mklink','/J',str(modules),str(shared)],check=True,capture_output=True)
if modules.resolve()!=shared.resolve():raise SystemExit('Worktree dependency path differs')
log=root/'target/no-public-live';log.mkdir(parents=True,exist_ok=True)
with (log/'vite.stdout.log').open('ab') as stdout,(log/'vite.stderr.log').open('ab') as stderr:
    proc=subprocess.Popen(['node',str(modules/'vite/bin/vite.js'),'--host','127.0.0.1','--port','5173','--strictPort'],cwd=root/'apps/desktop',stdout=stdout,stderr=stderr,creationflags=0x08000000)
(log/'vite-process.json').write_text(json.dumps({'pid':proc.pid,'source':str(root),'port':5173},indent=2))
print(proc.pid)
