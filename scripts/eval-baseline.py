import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import time
import urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('--data', type=Path, default=Path('/home/johan/neomem/eval-data'))
parser.add_argument('--models', type=Path)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--sample', type=int, default=7)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
os.environ['CARGO_TARGET_DIR'] = str(root / 'target')
if not os.environ.get('JEV_API_KEY'):
    for line in (Path.home() / '.neomem/secrets.env').open():
        if line.startswith('JEV_API_KEY='):
            os.environ['JEV_API_KEY'] = line.split('=', 1)[1].strip().strip('\"\'')
            break
if not os.environ.get('JEV_API_KEY'):
    raise SystemExit('JEV_API_KEY is missing')
args.out.mkdir(parents=True, exist_ok=False)
binaries = args.out / 'bin'
binaries.mkdir()
for name in ['neomem-eval', 'neomemd']:
    shutil.copy2(root / 'target/debug' / name, binaries / name)
(args.out / 'binaries.json').write_text(json.dumps({
    name: hashlib.file_digest((binaries / name).open('rb'), 'sha256').hexdigest()
    for name in ['neomem-eval', 'neomemd']
}, indent=2))
model_settings = json.loads((args.models or args.data / 'models.json').read_text())
server = model_settings['reader']['base_url'].removesuffix('/v1')
with urllib.request.urlopen(server + '/props') as response:
    (args.out / 'server-props.json').write_text(json.dumps(json.load(response), indent=2))
started = time.monotonic()
for name, dataset, sample in [
    ('longmemeval-s', args.data / 'longmemeval_s_cleaned.json', args.sample),
    ('coding', args.data / 'coding-set/questions.json', 50),
]:
    for arm in ['memory-off', 'search']:
        command = [str(binaries / 'neomem-eval'), 'run', str(dataset),
                   '--sample', str(sample), '--seed', '20261021', '--arm', arm,
                   '--models', str(args.models or args.data / 'models.json'),
                   '--out', str(args.out / f'{name}-{arm}')]
        with (args.out / f'{name}-{arm}.log').open('w') as log:
            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
(args.out / 'runtime.json').write_text(json.dumps({'seconds': time.monotonic() - started}))
