#!/usr/bin/env python3
import argparse
import json
import re
from datetime import datetime, timedelta
from pathlib import Path

import pyarrow.parquet as pq

SPLITS = ("Accurate_Retrieval", "Conflict_Resolution", "Long_Range_Understanding", "Test_Time_Learning")
INCLUDED = SPLITS[:2]


def sessions_of(context):
    start = 0
    while start < len(context):
        end = min(start + 4000, len(context))
        if end < len(context):
            boundaries = list(re.finditer(r"\n[ \t]*\n", context[start:end]))
            if boundaries:
                end = start + boundaries[-1].end()
            else:
                boundaries = list(re.finditer(r"\s+", context[start:end]))
                if boundaries:
                    end = start + boundaries[-1].end()
                else:
                    boundary = re.search(r"\s+", context[end:])
                    end = end + boundary.end() if boundary else len(context)
        yield context[start:end]
        start = end


def is_absent(answer):
    text = answer.strip().lower().rstrip(".!")
    return text in {"unknown", "unanswerable", "not mentioned", "not specified", "not provided", "no information"} or text.startswith(
        ("the information provided is not enough", "the context does not say", "the context does not mention", "the information is absent")
    )


def items_of(sample, split, row_index, limit):
    row_id = f"{split.lower()}-{row_index}"
    texts = list(sessions_of(sample["context"]))
    history = [[{"role": "document", "content": text}] for text in texts]
    session_ids = [f"{row_id}-S{index + 1}" for index in range(len(texts))]
    dates = [(datetime(2024, 1, 1, 9) + timedelta(minutes=index)).strftime("%Y/%m/%d (%a) %H:%M") for index in range(len(texts))]
    folded = [text.casefold() for text in texts]
    if len(sample["questions"]) != len(sample["answers"]):
        raise ValueError(f"{row_id}: questions and answers are not aligned")
    for index, (question, answer) in enumerate(zip(sample["questions"], sample["answers"])):
        if limit is not None and index >= limit:
            break
        answers = [str(value) for value in answer] if isinstance(answer, list) else [str(answer)]
        absent = bool(answers) and all(is_absent(value) for value in answers)
        evidence = [session_id for session_id, text in zip(session_ids, folded) if any(value and value.casefold() in text for value in answers)]
        yield {
            "question_id": f"{row_id}-q{index}" + ("_abs" if absent else ""),
            "question_type": f"{split.lower()}/{sample['metadata']['source']}",
            "question": question,
            "answer": " | ".join(dict.fromkeys(answers)),
            "question_date": dates[-1] if dates else "2024/01/01 (Mon) 09:00",
            "haystack_sessions": history,
            "haystack_session_ids": session_ids,
            "haystack_dates": dates,
            "answer_session_ids": evidence,
        }


def main():
    parser = argparse.ArgumentParser(description="Convert MemoryAgentBench (ai-hyz/MemoryAgentBench, MIT) into neomem-eval items")
    parser.add_argument("--max-questions-per-row", type=int)
    parser.add_argument("memoryagentbench", type=Path)
    parser.add_argument("out", type=Path)
    arguments = parser.parse_args()
    if arguments.max_questions_per_row is not None and arguments.max_questions_per_row < 0:
        parser.error("--max-questions-per-row must be nonnegative")
    count = 0
    row_count = 0
    with arguments.out.open("w") as output:
        output.write("[")
        for split in SPLITS:
            parquet = pq.ParquetFile(arguments.memoryagentbench / f"{split}.parquet")
            rows = 0
            questions = 0
            converted = 0
            columns = None if split in INCLUDED else ["questions"]
            for batch in parquet.iter_batches(batch_size=1, columns=columns):
                for sample in batch.to_pylist():
                    if split in INCLUDED:
                        for item in items_of(sample, split, rows, arguments.max_questions_per_row):
                            if count:
                                output.write(",")
                            json.dump(item, output)
                            count += 1
                            converted += 1
                        row_count += 1
                    rows += 1
                    questions += len(sample["questions"])
            if split in INCLUDED:
                print(f"{split}: {rows} rows, {questions} questions, {converted} converted")
            else:
                print(f"{split}: {rows} rows, {questions} questions skipped")
        output.write("]")
    print(f"{count} questions from {row_count} rows -> {arguments.out}")


if __name__ == "__main__":
    main()
