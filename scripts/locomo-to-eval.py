#!/usr/bin/env python3
import argparse
import json
import re
from datetime import datetime
from pathlib import Path

CATEGORIES = {1: "multi-hop", 2: "temporal-reasoning", 3: "open-domain", 4: "single-hop", 5: "adversarial"}


def session_date(text):
    moment = datetime.strptime(text.strip(), "%I:%M %p on %d %B, %Y")
    return moment.strftime("%Y/%m/%d (%a) %H:%M")


def turn_text(turn):
    text = turn["text"]
    if turn.get("blip_caption"):
        text += f" [shared a photo: {turn['blip_caption']}]"
    return text


def sessions_of(conversation):
    numbers = sorted(int(key.split("_")[1]) for key in conversation if re.fullmatch(r"session_\d+", key))
    return [number for number in numbers if conversation[f"session_{number}"]]


def items_of(sample):
    conversation = sample["conversation"]
    numbers = sessions_of(conversation)
    history = [[{"role": turn["speaker"], "content": turn_text(turn)} for turn in conversation[f"session_{n}"]] for n in numbers]
    session_ids = [f"{sample['sample_id']}-S{n}" for n in numbers]
    dates = [session_date(conversation[f"session_{n}_date_time"]) for n in numbers]
    for index, qa in enumerate(sample["qa"]):
        category = qa["category"]
        evidence = sorted({f"{sample['sample_id']}-S{ref.split(':')[0][1:]}" for ref in qa.get("evidence", []) if ref.startswith("D")})
        unanswerable = category == 5
        yield {
            "question_id": f"{sample['sample_id']}-q{index}" + ("_abs" if unanswerable else ""),
            "question_type": CATEGORIES[category],
            "question": qa["question"],
            "answer": "The conversations do not say." if unanswerable else str(qa["answer"]),
            "question_date": dates[-1],
            "haystack_sessions": history,
            "haystack_session_ids": session_ids,
            "haystack_dates": dates,
            "answer_session_ids": [] if unanswerable else [s for s in evidence if s in session_ids],
        }


def main():
    parser = argparse.ArgumentParser(description="Convert LoCoMo (snap-research/locomo) into neomem-eval items")
    parser.add_argument("locomo", type=Path)
    parser.add_argument("out", type=Path)
    arguments = parser.parse_args()
    samples = json.loads(arguments.locomo.read_text())
    items = [item for sample in samples for item in items_of(sample)]
    arguments.out.write_text(json.dumps(items))
    print(f"{len(items)} questions from {len(samples)} conversations -> {arguments.out}")


if __name__ == "__main__":
    main()
