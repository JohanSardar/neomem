#!/usr/bin/env python3
import argparse
import json
from pathlib import Path

import ijson


def main():
    parser = argparse.ArgumentParser(description='Check shared questions and histories against a legacy array (requires ijson)')
    parser.add_argument('legacy', type=Path)
    parser.add_argument('shared', type=Path)
    args = parser.parse_args()
    dataset = json.loads(args.shared.read_text())
    questions = {item['question_id']: item for item in dataset['questions']}
    count = 0
    with args.legacy.open('rb') as source:
        for item in ijson.items(source, 'item'):
            question = dict(questions.pop(item['question_id']))
            history = dataset['histories'][question.pop('history')]
            question['haystack_session_ids'] = history['session_ids']
            question['haystack_dates'] = history['dates']
            question['haystack_sessions'] = [dataset['sessions'][key] for key in history['session_ids']]
            if question != item:
                raise ValueError(f"Question or history differs: {item['question_id']}")
            count += 1
    if questions:
        raise ValueError('Extra questions in shared dataset')
    print(f'{args.shared}: {count} questions and all histories exactly match {args.legacy}')


if __name__ == '__main__':
    main()
