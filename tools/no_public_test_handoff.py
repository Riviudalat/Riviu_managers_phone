"""Offline subprocess inventory failures must never close or launch an app."""
import runpy
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

root=Path(__file__).parent
for name,arguments in [
    ('no_public_close.py',['--pid','123','--expected-path','C:/fixture.exe','--expected-start-ticks','100','--operator-confirmed-idle']),
    ('no_public_launch.py',['--exe',str((root/'no_public_close.py').resolve()),'--serial','fixture']),
]:
    with patch.object(sys,'argv',[name,*arguments]),patch.object(subprocess,'run',side_effect=subprocess.CalledProcessError(1,'inventory')),patch.object(subprocess,'Popen',side_effect=AssertionError('no launch')):
        try:runpy.run_path(str(root/name),run_name='__main__')
        except subprocess.CalledProcessError:pass
        else:raise AssertionError('Inventory failure was not propagated')
print('handoff inventory failure refuses close/launch: PASS')
