#!/usr/bin/env python3
import argparse
from eval_format import SharedDataset
import hashlib
import json
from datetime import datetime, timedelta
from pathlib import Path
from urllib.parse import quote
from urllib.request import urlopen

CATEGORIES = (
    "abstention_evidence",
    "assistant_facts_evidence",
    "changing_evidence",
    "implicit_connection_evidence",
    "preference_evidence",
    "user_evidence",
)
BASE = "https://huggingface.co/datasets/Salesforce/ConvoMem/resolve/main/"
TREE = "https://huggingface.co/api/datasets/Salesforce/ConvoMem/tree/main/"


def download(root, name):
    path = root / name
    if not path.exists():
        with urlopen(BASE + quote(name, safe="/"), timeout=120) as response:
            data = response.read()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return path


def files_of(root, category):
    prefix = f"core_benchmark/evidence_questions/{category}"
    url = TREE + prefix + "?recursive=1&limit=1000"
    page = 0
    while url:
        cache = root / "metadata" / f"{category}-{page}.json"
        if cache.exists():
            listing = json.loads(cache.read_text())
        else:
            with urlopen(url, timeout=120) as response:
                entries = json.load(response)
                links = response.headers.get("Link", "")
            following = next((part.split("<", 1)[1].split(">", 1)[0] for part in links.split(",") if 'rel="next"' in part), None)
            listing = {"entries": entries, "next": following}
            cache.parent.mkdir(parents=True, exist_ok=True)
            cache.write_text(json.dumps(listing))
        for entry in listing["entries"]:
            name = entry["path"]
            if entry["type"] == "file" and name.endswith(".json"):
                if not name.startswith(prefix + "/") or ".." in Path(name).parts:
                    raise ValueError(f"Unexpected dataset path: {name}")
                yield name
        url = listing["next"]
        page += 1


def items_of(sample, name, category):
    for index, qa in enumerate(sample["evidence_items"]):
        conversations = qa["conversations"]
        history = [[{"role": turn["speaker"].lower(), "content": turn["text"]} for turn in conversation["messages"]] for conversation in conversations]
        if not history or any(not session for session in history):
            raise ValueError(f"Empty conversation history: {name}:{index}")
        canonical = json.dumps(history, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        history_id = hashlib.sha256(canonical.encode()).hexdigest()
        session_ids = [f"convomem-{history_id}-S{n + 1}" for n in range(len(history))]
        dates = [(datetime(2024, 1, 1) + timedelta(days=n)).strftime("%Y/%m/%d (%a) %H:%M") for n in range(len(history))]
        unanswerable = category == "abstention_evidence"
        evidence = [session_id for session_id, conversation in zip(session_ids, conversations) if conversation.get("containsEvidence", False)]
        if not unanswerable and not evidence:
            raise ValueError(f"No evidence sessions: {name}:{index}")
        yield {
            "question_id": f"convomem-{name.removesuffix('.json')}-q{index}" + ("_abs" if unanswerable else ""),
            "question_type": category,
            "question": qa["question"],
            "answer": str(qa["answer"]),
            "question_date": dates[-1],
            "haystack_sessions": history,
            "haystack_session_ids": session_ids,
            "haystack_dates": dates,
            "answer_session_ids": [] if unanswerable else evidence,
        }


def main():
    parser = argparse.ArgumentParser(description="Convert Salesforce/ConvoMem (CC BY-NC 4.0) into neomem-eval items")
    parser.add_argument("--limit-per-category", type=int)
    parser.add_argument("--from-eval", type=Path, help="convert an existing array without downloading source files")
    parser.add_argument("convomem", type=Path)
    parser.add_argument("out", type=Path)
    arguments = parser.parse_args()
    if arguments.from_eval:
        dataset = SharedDataset()
        for item in json.loads(arguments.from_eval.read_text()):
            dataset.add(item)
        dataset.write(arguments.out)
        return
    if arguments.limit_per_category is not None and arguments.limit_per_category < 1:
        parser.error("--limit-per-category must be positive")
    arguments.convomem.mkdir(parents=True, exist_ok=True)
    download(arguments.convomem, "README.md")
    items = []
    for category in CATEGORIES:
        count = 0
        for name in files_of(arguments.convomem, category):
            sample = json.loads(download(arguments.convomem, name).read_text())
            for item in items_of(sample, name, category):
                items.append(item)
                count += 1
                if count == arguments.limit_per_category:
                    break
            if count == arguments.limit_per_category:
                break
        print(f"{category}: {count} questions")
    arguments.out.parent.mkdir(parents=True, exist_ok=True)
    dataset = SharedDataset()
    for item in items:
        dataset.add(item)
    dataset.write(arguments.out)
    histories = {tuple(item["haystack_session_ids"]) for item in items}
    print(f"{len(items)} questions from {len(histories)} distinct histories -> {arguments.out}")


if __name__ == "__main__":
    main()
