"""Generate clearly local, unsent test media; no device or network access."""
from pathlib import Path
import hashlib
import json
from PIL import Image, ImageDraw

root = Path('target/no-public-fixture')
root.mkdir(parents=True, exist_ok=False)
bundle = root / 'rehearsal-photo'
bundle.mkdir()
for index,color in enumerate(['#E9F0F7','#FFF0DD'],1):
    image = Image.new('RGB',(1080,1440),color)
    draw = ImageDraw.Draw(image)
    draw.rectangle((80,80,1000,1360),outline='#C2410C',width=10)
    draw.text((160,620),f'LOCAL DRAFT REHEARSAL {index}',fill='#243447',stroke_width=2)
    draw.text((160,720),'NOT FOR PUBLICATION',fill='#243447')
    image.save(bundle/f'{index:02}.jpg',quality=92)
(bundle/'caption.txt').write_text('Bản nháp kiểm tra cục bộ — không đăng công khai.',encoding='utf8')
records={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in bundle.iterdir() if p.is_file()}
(root/'input-hashes.json').write_text(json.dumps(records,indent=2),encoding='utf8')
print(root.resolve())
