import argparse
import concurrent.futures
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
    'memoryagentbench': 'memoryagentbench/mab-eval.v2.json',
    'beam-100k': 'beam/beam-100k-eval.json',
    'longmemeval-m': 'longmemeval-m/longmemeval-m-eval.v2.json',
    'locomo-v2': 'locomo/locomo-eval.v2.json',
    'convomem-v2': 'convomem/convomem-eval.v2.json',
    'memoryagentbench-v2': 'memoryagentbench/mab-eval.v2.json',
    'beam-100k-v2': 'beam/beam-100k-eval.v2.json',
    'beam-500k': 'beam/beam-500k-eval.v2.json',
    'beam-1m': 'beam/beam-1m-eval.v2.json',
    'beam-10m': 'beam/beam-10m-eval.v2.json',
    'coding': 'coding-set/questions.json',
}
CODING_QUESTIONS = 50
BRIDGE_PORT = 18973
ATTEMPTS = 4
RETRY_PAUSE = 60

parser = argparse.ArgumentParser()
parser.add_argument('--data', type=Path, default=Path('/home/johan/neomem/eval-data'))
parser.add_argument('--models', type=Path)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--sample', type=int, default=7)
parser.add_argument('--benchmarks', default='longmemeval-s,coding')
parser.add_argument('--arms', default='memory-off,search')
parser.add_argument('--held-out', action='store_true')
parser.add_argument('--parallel', type=int, default=1, help='shards run side by side, each in its own daemon home')
parser.add_argument('--binaries', type=Path, default=Path('target/debug'),
                    help='build folder holding neomem-eval and neomemd, relative to the repository')
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
    shutil.copy2(root / args.binaries / name, binaries / name)
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
(args.out / 'source.json').write_text(json.dumps({'source_revision_at_freeze': source_revision,
                                                   'binaries': str(args.binaries)}, indent=2))
chosen = args.benchmarks.split(',')
inputs = [args.data / BENCHMARKS[name] for name in chosen]
if 'coding' in chosen:
    inputs.extend(sorted((args.data / 'coding-set').glob('q*.jsonl')))
(args.out / 'data-files.json').write_text(json.dumps({
    str(path.relative_to(args.data)): hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()
    for path in inputs
}, indent=2))


def shard_command(name, arm, shard, attempt):
    sample = CODING_QUESTIONS if name == 'coding' else args.sample
    dataset = (args.out / 'prepared' / name / f'shard-{shard}.json'
               if args.parallel > 1 else args.data / BENCHMARKS[name])
    command = [str(binaries / 'neomem-eval'), 'run', str(dataset),
               '--arm', arm, '--models', str(models),
               '--out', str(args.out / f'{name}-{arm}' / f'shard-{shard}')]
    if args.parallel > 1:
        command.append('--prepared')
    else:
        command.extend(['--sample', str(sample), '--seed', '20261021'])
        if args.held_out:
            command.append('--held-out')
    if attempt > 0:
        command.append('--resume')
    return command


def run_shard(name, arm, shard):
    for attempt in range(ATTEMPTS):
        with (args.out / f'{name}-{arm}-shard-{shard}.log').open('a') as log:
            log.write(f'--- attempt {attempt + 1} at {time.strftime("%H:%M:%S")}\n')
            log.flush()
            command = shard_command(name, arm, shard, attempt)
            if subprocess.run(command, stdout=log, stderr=subprocess.STDOUT).returncode == 0:
                return
        time.sleep(RETRY_PAUSE)
    raise RuntimeError(f'{name} {arm}: shard {shard} failed {ATTEMPTS} times')


def run_shards(name, arm):
    with concurrent.futures.ThreadPoolExecutor(args.parallel) as pool:
        for future in [pool.submit(run_shard, name, arm, shard) for shard in range(args.parallel)]:
            future.result()


def merge(name, arm):
    directory = args.out / f'{name}-{arm}'
    shards = [directory / f'shard-{shard}' for shard in range(args.parallel)]
    manifests = [json.loads((shard / 'manifest.json').read_text()) for shard in shards]
    manifest = dict(manifests[0])
    manifest['selected'] = [question for each in manifests for question in each['selected']]
    manifest.pop('shard')
    items = [item for shard in shards for item in json.loads((shard / 'items.json').read_text())]
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2))
    (directory / 'items.json').write_text(json.dumps(items, indent=2))
    subprocess.run([str(binaries / 'neomem-eval'), 'report', str(directory)], check=True)


started = time.monotonic()
try:
    for name in chosen:
        if args.parallel > 1:
            prepared = args.out / 'prepared' / name
            command = [str(binaries / 'neomem-eval'), 'prepare', str(args.data / BENCHMARKS[name]),
                       '--sample', str(CODING_QUESTIONS if name == 'coding' else args.sample),
                       '--seed', '20261021', '--shards', str(args.parallel), '--out', str(prepared)]
            if args.held_out:
                command.append('--held-out')
            subprocess.run(command, check=True)
        for arm in args.arms.split(','):
            run_shards(name, arm)
            merge(name, arm)
finally:
    if bridge:
        bridge.terminate()
(args.out / 'runtime.json').write_text(json.dumps({'seconds': time.monotonic() - started}))
