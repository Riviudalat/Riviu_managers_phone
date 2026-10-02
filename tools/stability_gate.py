"""Run final-source gates serially and retain logs outside ephemeral task output."""
from pathlib import Path
import hashlib
import json
import subprocess
import sys

root=Path.cwd();out=root/'target/stability-gates';out.mkdir(parents=True,exist_ok=True)
commands=[
 ['cargo','test','--offline','--locked','-p','riviu-core','interaction_hierarchy','--lib','--','--test-threads=1'],
 ['cargo','test','--offline','--locked','-p','riviu-core','interaction_campaign','--lib','--','--test-threads=1'],
 ['cargo','test','--offline','--locked','-p','riviu-core','rehearsal','--lib','--','--test-threads=1'],
 ['cargo','test','--offline','--locked','-p','riviu-core','tiktok_sound','--lib','--','--test-threads=1'],
 ['cargo','test','--offline','--locked','-p','riviu-managers-phone','no_public','--lib','--','--test-threads=1'],
 ['cargo','build','--offline','--locked','-p','riviu-managers-phone','--bin','riviu-managers-phone'],
]
files=['crates/core/src/interaction_hierarchy.rs','crates/core/src/interaction_hierarchy/draft_cleanup.rs','crates/core/src/interaction_campaign.rs','crates/core/src/interaction_target.rs','apps/desktop/src-tauri/src/lib.rs','apps/desktop/src-tauri/src/no_public.rs']
records=[]
for index,command in enumerate(commands):
    snapshot={file:hashlib.sha256((root/file).read_bytes()).hexdigest() for file in files}
    log=out/f'gate-{index:02}.log'
    with log.open('wb') as stream:result=subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT)
    unchanged=all(hashlib.sha256((root/file).read_bytes()).hexdigest()==digest for file,digest in snapshot.items())
    record={'command':command,'exitCode':result.returncode,'log':str(log),'sourceUnchanged':unchanged,'sourceHashes':snapshot}
    records.append(record);(out/'results.json').write_text(json.dumps(records,indent=2))
    print(json.dumps({'gate':index,'exitCode':result.returncode,'sourceUnchanged':unchanged,'log':str(log)}),flush=True)
    if result.returncode or not unchanged:sys.exit(result.returncode or 1)
