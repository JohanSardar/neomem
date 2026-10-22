import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import time
import urllib.request

BENCHMARKS = {
    'longmemeval-s': 'longmemeval_s_cleaned.json',
    'locomo': 'locomo/locomo-eval.json',
    'convomem': 'convomem/convomem-eval.json',
    'memoryagentbench': 'memoryagentbench/mab-eval.json',
    'beam-100k': 'beam/beam-100k-eval.json',
    'coding': 'coding-set/questions.json',
}
CODING_QUESTIONS = 50
BRIDGE_PORT = 18973

parser = argparse.ArgumentParser()
parser.add_argument('--data', type=Path, default=Path('/home/johan/neomem/eval-data'))
parser.add_argument('--models', type=Path)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--sample', type=int, default=7)
parser.add_argument('--benchmarks', default='longmemeval-s,coding')
parser.add_argument('--arms', default='memory-off,search')
parser.add_argument('--held-out', action='store_true')
parser.add_argument('--bridge', type=Path, help='secrets.env with a neomem ChatGPT sign-in; serves the reader and judge')
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
models = args.models or args.data / 'models.json'
model_settings = json.loads(models.read_text())
bridge = None
if args.bridge:
    bridge_binary = root / 'target/release/examples/chat_completions_bridge'
    bridge = subprocess.Popen([str(bridge_binary), str(args.bridge), str(BRIDGE_PORT)],
                              stdout=(args.out / 'bridge.log').open('w'), stderr=subprocess.STDOUT)
    time.sleep(1)
server = model_settings['reader']['base_url'].removesuffix('/v1')
try:
    with urllib.request.urlopen(server + '/props', timeout=10) as response:
        (args.out / 'server-props.json').write_text(json.dumps(json.load(response), indent=2))
except OSError:
    pass
source_revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
(args.out / 'source.json').write_text(json.dumps({'source_revision_at_freeze': source_revision}, indent=2))
chosen = args.benchmarks.split(',')
inputs = [args.data / BENCHMARKS[name] for name in chosen]
if 'coding' in chosen:
    inputs.extend(sorted((args.data / 'coding-set').glob('q*.jsonl')))
(args.out / 'data-files.json').write_text(json.dumps({
    str(path.relative_to(args.data)): hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()
    for path in inputs
}, indent=2))
started = time.monotonic()
try:
    for name in chosen:
        sample = CODING_QUESTIONS if name == 'coding' else args.sample
        for arm in args.arms.split(','):
            command = [str(binaries / 'neomem-eval'), 'run', str(args.data / BENCHMARKS[name]),
                       '--sample', str(sample), '--seed', '20261021', '--arm', arm,
                       '--models', str(models), '--out', str(args.out / f'{name}-{arm}')]
            if args.held_out:
                command.append('--held-out')
            with (args.out / f'{name}-{arm}.log').open('w') as log:
                subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
finally:
    if bridge:
        bridge.terminate()
(args.out / 'runtime.json').write_text(json.dumps({'seconds': time.monotonic() - started}))
