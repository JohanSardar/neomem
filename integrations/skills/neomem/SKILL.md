---
name: neomem
description: Long-term memory shared by every agent session on this machine. Search it before work that may build on earlier decisions, people, projects, errors or preferences, and whenever the user refers to the past ("like last time", "we decided", "again") or names someone or something you do not know. Cite the fct_ IDs you use.
---

# neomem

neomem remembers facts from earlier agent sessions and from other sources such as calendars, notes and purchases. A local daemon answers the `neomem` command in a few milliseconds. Every command accepts `--json`.

## When to search

- At the start of a task that may depend on earlier work, decisions, people or preferences.
- When the user mentions the past, a name you have not seen in this session, or an error that may have happened before.
- Not for follow-ups inside the task you are already doing.

## Search facts

```bash
neomem search "Edge ship date"
neomem search "Sam Kim role" --k 5
neomem search "code signing error" --tags code --from 2026-09-01
neomem search "Edge ship date" --as-of 2026-09-15
neomem search "Edge ship date" --include-invalid
```

- Write the query the way the fact would be written ("Edge ship date"), not as a question.
- Results show the latest version of each fact; replaced versions are hidden. When two results are flagged as contradicting, say so instead of picking one.
- `--as-of <date>` answers what was true on that date. `--include-invalid` also shows replaced facts.
- `--tags` and `--entities` take comma-separated lists; `--from` and `--to` limit the time.

## Look closer

```bash
neomem show <fact-id>
neomem history <fact-id>
neomem entity "Edge"
neomem resolve "Sam Kim"
neomem raw "signing" --from 2026-09-01
```

| Command | Answers |
|---|---|
| `neomem show <id>` | the full memory and the records it came from |
| `neomem history <fact-id>` | every version of a fact, oldest first |
| `neomem entity <name>` | a person, project or thing: aliases, facts, linked entities |
| `neomem resolve <name>` | the entity ID for a name or alias |
| `neomem raw <words>` | keyword search over the raw history, no ranking ("what did I say on Tuesday?") |

## Give feedback and write

```bash
neomem useful <fact-id>
neomem add "Maya prefers meetings before noon" --source agent --tags social
neomem correct <intake-id> --reason "logged by mistake"
```

- Run `neomem useful <fact-id>` when a memory actually helped; it keeps that memory from fading.
- This session is captured automatically. Use `neomem add` only when the user asks you to remember something that is not in this session, or to import data.
- `neomem correct <int_ id>` marks a raw record as logged wrongly; facts built on it are checked again. `neomem show` lists a fact's int_ sources.

## Rules

- Cite the fct_ ID of every memory you rely on.
- A memory can be out of date. What the user says now and the current code win; when they disagree with a memory, say so.
- Never store secrets, tokens or passwords.
- If `neomem` reports that the daemon is not running, tell the user once and continue without memory.
