#!/usr/bin/env python3
import argparse
from eval_format import SharedDataset
import ast
import json
import re
import urllib.request
from collections import Counter
from datetime import datetime, timedelta
from itertools import zip_longest
from pathlib import Path

import pyarrow.parquet as parquet

DATASET = "https://huggingface.co/datasets/Mohammadta/BEAM/resolve/main/data"
DOWNLOAD_ROOT = Path("/home/johan/neomem/eval-data/beam")
DATE_FORMAT = "%Y/%m/%d (%a) %H:%M"


def session_date(session, index):
    anchor = next((turn["time_anchor"] for turn in session if turn.get("time_anchor")), None)
    if anchor:
        for pattern in ("%B-%d-%Y", "%B %d, %Y", "%Y-%m-%d", DATE_FORMAT):
            try:
                return datetime.strptime(anchor, pattern)
            except ValueError:
                pass
        return datetime.fromisoformat(anchor.replace("Z", "+00:00")).replace(tzinfo=None)
    return datetime(2024, 1, 1) + timedelta(days=index)


def readable(value):
    if isinstance(value, dict):
        return "; ".join(f"{key}: {readable(part)}" for key, part in value.items())
    if isinstance(value, list):
        return "; ".join(readable(part) for part in value)
    return str(value)


def evidence_ids(value):
    if isinstance(value, dict):
        for part in value.values():
            yield from evidence_ids(part)
    elif isinstance(value, list):
        for part in value:
            yield from evidence_ids(part)
    else:
        yield str(value)


def sessions_of(chat):
    if not chat or isinstance(chat[0], list):
        return [session for session in chat if session]
    sessions = []
    for plan in chat:
        for batches in plan.values():
            for batch in batches or []:
                sessions.extend(session for session in batch["turns"] if session)
    return sessions


def items_of(sample, size, limit):
    conversation_id = f"beam-{size}-{sample['conversation_id']}"
    sessions = sessions_of(sample["chat"])
    history = [[{"role": turn["role"], "content": turn["content"]} for turn in session] for session in sessions]
    session_ids = [f"{conversation_id}-S{index + 1}" for index in range(len(sessions))]
    moments = [session_date(session, index) for index, session in enumerate(sessions)]
    dates = [moment.strftime(DATE_FORMAT) for moment in moments]
    question_date = (max(moments) + timedelta(days=1)).strftime(DATE_FORMAT)
    turn_sessions = {str(turn["id"]): session_id for session, session_id in zip(sessions, session_ids) for turn in session}
    raw = sample["probing_questions"]
    try:
        probes = json.loads(raw)
    except json.JSONDecodeError:
        probes = ast.literal_eval(raw)
    groups = [[(category, index, qa) for index, qa in enumerate(questions)] for category, questions in probes.items()]
    questions = [entry for batch in zip_longest(*groups) for entry in batch if entry is not None]
    if limit is not None:
        questions = questions[:limit]
    for category, index, qa in questions:
        answer = next((qa[key] for key in ("answer", "ideal_answer", "ideal_response", "ideal_summary", "rubric", "expected_compliance") if qa.get(key)), None)
        if answer is None:
            raise ValueError(f"Missing answer: {conversation_id}/{category}/{index}")
        answer = readable(answer)
        abstention = category == "abstention" or bool(qa.get("abstention_type")) or bool(re.search(r"\b(?:never (?:mentioned|provided|stated|discussed)|not (?:mentioned|provided|stated|discussed)|no information|does not (?:mention|provide|state))\b", answer, re.IGNORECASE))
        evidence = set(evidence_ids(qa.get("source_chat_ids", [])))
        evidence_sessions = {turn_sessions[ref] for ref in evidence if ref in turn_sessions}
        yield {
            "question_id": f"{conversation_id}-{category}-q{index}" + ("_abs" if abstention else ""),
            "question_type": category,
            "question": qa["question"],
            "answer": answer,
            "question_date": question_date,
            "haystack_sessions": history,
            "haystack_session_ids": session_ids,
            "haystack_dates": dates,
            "answer_session_ids": [session_id for session_id in session_ids if session_id in evidence_sessions],
        }


def main():
    parser = argparse.ArgumentParser(description="Convert BEAM (Mohammadta/BEAM, CC BY-SA 4.0) into neomem-eval items")
    parser.add_argument("--size", choices=("100K", "500K", "1M", "10M"), default="100K")
    parser.add_argument("--max-questions-per-conversation", type=int)
    parser.add_argument("beam", type=Path)
    parser.add_argument("out", type=Path)
    arguments = parser.parse_args()
    if arguments.max_questions_per_conversation is not None and arguments.max_questions_per_conversation < 1:
        parser.error("--max-questions-per-conversation must be positive")
    if not arguments.beam.resolve().is_relative_to(DOWNLOAD_ROOT.resolve()):
        parser.error(f"download directory must be within {DOWNLOAD_ROOT}")
    arguments.beam.mkdir(parents=True, exist_ok=True)
    filenames = ([f"10M-{index:05}-of-00002.parquet" for index in range(2)]
                 if arguments.size == "10M" else [f"{arguments.size}-00000-of-00001.parquet"])
    samples = []
    for filename in filenames:
        source = arguments.beam / filename
        if not source.exists():
            base = ("https://huggingface.co/datasets/Mohammadta/BEAM-10M/resolve/main/data"
                    if arguments.size == "10M" else DATASET)
            urllib.request.urlretrieve(f"{base}/{filename}", source)
        for batch in parquet.ParquetFile(source).iter_batches(batch_size=1, columns=["conversation_id", "chat", "probing_questions"]):
            samples.extend(batch.to_pylist())
    items = [item for sample in samples for item in items_of(sample, arguments.size, arguments.max_questions_per_conversation)]
    if len({item["question_id"] for item in items}) != len(items):
        raise ValueError("Duplicate question IDs")
    dataset = SharedDataset()
    for item in items:
        dataset.add(item)
    dataset.write(arguments.out)
    turns = sum(len(session) for sample in samples for session in sessions_of(sample["chat"]))
    characters = sum(len(turn["content"]) for sample in samples for session in sessions_of(sample["chat"]) for turn in session)
    print(f"{len(items)} questions from {len(samples)} conversations -> {arguments.out}")
    print(f"Total turns: {turns}")
    print(f"Approximate characters: {characters}")
    print("Questions per type:")
    for category, count in sorted(Counter(item["question_type"] for item in items).items()):
        print(f"  {category}: {count}")


if __name__ == "__main__":
    main()
