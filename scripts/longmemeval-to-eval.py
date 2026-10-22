#!/usr/bin/env python3
import argparse

import ijson

from eval_format import SharedDataset
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description='Convert LongMemEval S/M to shared neomem-eval/2 (requires ijson)')
    parser.add_argument('source', type=Path)
    parser.add_argument('out', type=Path)
    args = parser.parse_args()
    dataset = SharedDataset()
    with args.source.open('rb') as source:
        for item in ijson.items(source, 'item'):
            dataset.add(item)
    dataset.write(args.out)


if __name__ == '__main__':
    main()
