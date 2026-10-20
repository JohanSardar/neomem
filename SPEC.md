# neomem — specification

Status: design agreed 2026-10-19 (revision 3: semantic-only, SQLite storage, module design). Nothing built yet.
License: AGPL-3.0 (code and prompts are copied from Honcho and Basic Memory, which are AGPL).

neomem is a local, long-term **semantic** memory that any agent (Claude Code, Codex, pi, and a custom harness built on pi) can write to and search, for any user. It stores durable **facts** about **entities**, linked in a graph and embedded for meaning search, with Markdown files as the master copy and a rebuildable SQLite + in-memory index for speed. It works the way Honcho and the other memory systems do semantic memory.

**There is no episodic layer.** Episodes may live in a separate system later, which plugs into neomem through a shared contract (§11) of IDs, tags, link types and entity lookup.

### Revision 3 changes (from revision 2)
- **Storage is SQLite plus in-memory indexes** (§9). Postgres and Neo4j were dropped: no database services, about 2 GB less RAM, and no network round trip on the search path. They can come back later as index adapters if scale needs them (§13.3).
- **Module design added** (§13): a thin `neomem` client talks to the always-running `neomemd` over a local socket; the code is a Rust workspace of deep modules with seams only where something really varies (models, agents, transports).
- **Every model is swappable by config**: embedding model, judge, planner (query rewrite), extractor and consolidation models (§13.4). Swapping the embedding model re-embeds in the background and switches over when done.
- **Local models run in-process with llama.cpp on Metal** (GGUF), and are unloaded after 15 minutes without a search or recall.
- **Cloud models work with any provider** (§13.4): OpenAI, OpenRouter, Anthropic, Hugging Face, Gemini, Groq, Together, Ollama, LM Studio, vLLM, your ChatGPT login (directly, as pi and Hermes do, or through Codex) or any command-line tool. The recall planner defaults to **GPT-6.1-Sol on low effort through the ChatGPT login** (~2 s).
- Codex hooks can return additional context, so recall mode works on Codex too (§10.3).
- **How it's built** (§17): poteto's pstack method. A **feature graph** (the feature map and the build plan in one) plus a **control CLI** (`neomemctl`) let agents verify their own work with proof; a feature is done only when its proof passes. No code comments.
- **The judge is Jev** (TypeSafe's "System One" decision model, hosted API): typed decisions with calibrated probabilities in ~70–500 ms. It runs the recall gate and approval, and the write-time same / extends / updates / contradicts / new check. A local reranker is the offline fallback. Jev can't write text, so the planner stays a text model (§10.3).

### Revision 2 changes (from revision 1)
- Episodes, composite episodes, the episode composer, the `timeline` tool and the "hide fragments / corrected episodes" rules were removed.
- Facts now link `derived_from` → the **intake record** (the exact message, turn or report), the way Honcho's conclusions point at source messages.
- Raw history became searchable (`neomem raw`), as Honcho's agents can search raw messages.
- Other systems can attach **external references** to neomem entities and facts (§11).
- The **shared contract** was added (§11).
- **Search is a fast, fixed pipeline, never an agent** (§10.2): vector top-k, swap every hit for the latest version of its fact, add useful linked memories, return. No model is called.
- **Recall mode** was added (§10.3): an optional automatic path. A judge model decides whether a turn needs memory, a text model rewrites the session into search queries, the search pipeline runs, the judge approves each memory, and only approved memories are injected.
- Future plans are facts. When a plan happens or changes, its fact is updated, and an event entity is created if one is useful.

---

## 1. Principles

1. **Nothing is ever deleted.** Outdated facts are marked invalid, not removed. Intake records are never edited. Decay only lowers ranking.
2. **No inference.** neomem records what was said or directly observed. It never writes deductions, inductions, traits, motives, patterns or predictions. The only links it creates between memories are structural (`updates`, `extends`, `part_of`, `contradicts`, `derived_from`).
3. **Every memory links to where it came from**: the intake record (transcript turn, report, message) it was drawn from.
4. **Files are the master copy.** Every fact and entity is a readable Markdown file. The search index (`index.db` plus the daemon's in-memory indexes) can be thrown away and rebuilt.
5. **Quality over speed.** Saving is instant. Processing runs in the background and may take its time.
6. **Local and contained.** It runs on the user's machine. Sensitive data never goes into logs, remote git, or anywhere it doesn't belong.
7. **Works with any agent.** The main interface is a command-line tool that any agent with a shell can use. MCP is offered on top.
8. **Shareable.** IDs, tags and link types form a published contract other systems can build on.
9. **English only.**

## 2. What goes in

### 2.1 Agent sessions (captured automatically)
Claude Code, Codex, pi, and the custom pi harness. See §7.

### 2.2 Everything else (written in by automations)
Raw data (health, money, email, texts, screen time, call logs, calendars, analytics, school, social, logistics, …) **lives in its own systems, not in neomem.** The user sets up **automations**: scheduled scripts or agents of the user's choosing that read those sources and write digested items into neomem. For example, a job that reads Oura data every morning and writes one sleep report.

- Automations are outside neomem. They write with `neomem add` (or HTTP or MCP) and may read with `neomem search`.
- An automation that runs inside an agent session is **captured in full**, like any other session (§7.4).
- Any future source plugs in this way, with no change to neomem.

The full list of sources this is meant to cover, from the requirements doc:
- conversations with agents
- agent actions
- coding sessions
- contacts and conversations with other people
- calendars
- reminders, notes and documents
- computer and phone activity
- health
- money
- work and business tools
- business analytics
- school
- social media
- life logistics
- any future source

### 2.3 Intake record format
Every write to neomem, from an automation, an agent or a hook, is one intake record:

```yaml
source:       "claude-code" | "codex" | "pi" | "oura-digest" | "bank-digest" | ...
occurred:     { start: <epoch ms>, end: <epoch ms> }   # the time span the content describes
tags:         [health, sleep]
content:      "free text or markdown (the report / message / event)"
entity_hints: ["Johan", "Oura"]        # optional
source_ref:   "uri back to the original" # optional
```

neomem works out the facts, the dates inside the content and the entities by itself. `content` is free text, so no source needs a schema.

### 2.4 Intake log (the raw history)
Every intake record gets an `int_` ID and is copied **exactly and permanently** into the intake log (in `state.db`, §9.3) the moment it arrives, before any processing. The intake log is:
- **the provenance** for every fact (`derived_from`)
- **the raw history agents can search** with `neomem raw` (§10). This plays the role of Honcho's message search.
- **the input for re-running extraction** with a better model, or rebuilding everything

There are **no dedupe keys**. If something is sent twice, it is logged twice. Its facts still don't duplicate (§5 step 4).

### 2.5 Late and out-of-order data
Data can arrive late and in any order. A fact's time range comes from what the content says and from `occurred`, not from when it arrived. If late data touches facts that were already reconciled, those facts are queued for re-consolidation (§6.3).

### 2.6 Corrections
- **Wrong facts** are corrected the normal semantic way: a new fact with an `updates` link, and the old fact gets `invalid_at`.
- **A wrong intake record** (an automation sent the wrong night's report, a message was logged incorrectly) is never edited. A **correction record** marks it as wrong (`corrects` →). Facts drawn from it are queued for re-checking, and re-running extraction skips it.

## 3. Memory model

There are two kinds of memory, **facts** and **entities**, plus **tags** and **links**. Intake records are raw history, not memories.

### 3.1 Timestamps
All timestamps are stored as **whole-number epoch milliseconds**.

| Field | On | Meaning |
|---|---|---|
| `recorded_at` | facts, entities, intake records | when neomem first stored it |
| `updated_at` | facts, entities | last change to the memory's metadata (links, importance, invalidation) |
| `occurred_start` / `occurred_end` | intake records | the time span the content describes |
| `valid_at` / `invalid_at` | facts, entity relations | when the fact became true / stopped being true (`invalid_at = null` means currently true) |

### 3.2 Facts: what is true
- Durable facts and relations, e.g. "Edge's ship date is November 1, 2026", "Sam works at Acme".
- **Every fact links to at least one entity** it is about. There are no peers and no record of who believes what; a fact only records which entities it's about.
- **Every fact links `derived_from` → the intake record(s)** it came from. More sources get attached as the same fact is seen again.
- **Allowed sources:** an explicit statement ("Johan said he switched to Puma") or a directly observed state ("Johan owns a 2019 MacBook", from a receipt).
- **Never inferred:** "prefers working late", "is stressed", or "likes Puma" taken from a purchase alone are all banned. Repetition only raises a fact's evidence and importance; it never creates a new conclusion.
- Facts record time ranges (`valid_at` / `invalid_at`), so the right fact is available for any point in time.
- **Updates:** when a fact changes, the new fact links to the old one with `updates`. The old fact gets `invalid_at` set and is kept, with its file, forever.
- **Extends:** a fact that adds detail links to the original with `extends`. Both stay valid.
- **Contradictions** the system can't resolve get a `contradicts` link and are kept until resolved.
- **Future plans are facts** ("dentist with Dr. Lee, Thu 15:00"). If the plan moves, the new fact `updates` it. Once it happens, an event entity is created if one is useful.
- Facts are embedded, are in the graph, and **are the only memories that decay** (§8).
- Each fact has an **importance** score from 1 to 10, set by the model when it's extracted.
- **No expiry dates.**

### 3.3 Entities: what memories are about
- Any person, agent, organization, codebase, place, thing, idea, product, event, … The `kind` list is **open, not a fixed set**.
- **Created by the system as needed:** when something is named and either is referred to by at least 2 facts or is clearly a person, organization, codebase or event.
- **Before creating one**, the system checks for an existing match by name and by meaning, and links to that instead if found.
- **Agents** are entities identified by product plus owner ("Johan's Claude Code", "Sam's assistant").
- **Merging:**
  - Merge automatically when the evidence is strong (a contact card listing both handles, an exact email match).
  - Keep every name or handle as an **alias**, and record `merged_from`, so any merge can be undone.
  - Weak matches get a `possibly_same` link for review.
  - **A merged entity's old ID keeps resolving to the survivor**, so external systems never break (§11).
- Entities **do not decay**.
- Relations between entities (`works_at`, `sister_of`, `owns_repo`, …) are graph links with **open, model-written labels**, and each carries `valid_at` / `invalid_at`. `part_of` is used between entities too ("Paywall is part of Edge").

### 3.4 Tags
- Labels that describe a memory's kind or topic. **Tags are not entities.**
- **Starting set:** `conversation`, `agent-session`, `work-session`, `code`, `health`, `sleep`, `money`, `purchase`, `calendar`, `social`, `school`, `travel`, `place`, `media`, `raw`, `system`.
- **New tags:** the model, or an external system, may create one through neomem. Each candidate is first compared by meaning against existing tags, so near-duplicates (`health` / `healthcare` / `wellness`) aren't created.
- neomem owns the **tag registry**, and other systems read it (§11).
- Search can filter by tag. `raw` and `system` are hidden by default.

### 3.5 Link types (fixed set)

| Link | From → to | Meaning |
|---|---|---|
| `updates` | new fact → old fact | replaces it; the old fact becomes invalid |
| `extends` | new fact → fact | adds detail; both stay valid |
| `contradicts` | fact ↔ fact | an unresolved conflict, kept until resolved |
| `part_of` | entity → entity, fact → fact | a part belongs to a larger whole |
| `derived_from` | fact → intake record | where it came from (provenance, not reasoning) |
| `about` | fact → entity | which entities it concerns |
| `possibly_same` | entity ↔ entity | candidate for a merge, awaiting review |
| `corrects` | correction record → intake record | the record was logged incorrectly |
| `mentions` (and other external types) | external reference → entity / fact | another system's object points here (§11) |

Entity-to-entity relations use open labels (§3.3).

## 4. Readable files (the master copy)

One Markdown file per fact and entity, in the format Basic Memory uses: a YAML header with all metadata, then the content as plain text.

```
~/.neomem/
  facts/2026/10/<id>_edge-ship-date-nov-1.md    # grouped by when it was recorded
  entities/<kind>/sam-kim.md                     # aliases, merged_from, links, external refs
  state.db                                       # SQLite: intake log + operational state (§9.3), cannot be rebuilt
  index.db                                       # SQLite: search index (§9.1), rebuildable, not backed up
  config.toml                                    # models, recall switches, limits (§13.4)
  models/                                        # local GGUF model files
  spool/                                         # hook signals written while the daemon was down (§13.1)
  run/neomemd.sock                               # the daemon's local socket (§13.1)
```

```markdown
---
id: fct_01JB…
kind: fact
tags: [code, edge]
about: [ent_edge]
recorded_at: 1760796300000
updated_at: 1760796300000
valid_at: 1760745600000
invalid_at: null
importance: 7
sources: [int_01JA…]
links: [{type: updates, to: fct_01J9…}]
---
Edge's ship date is November 1, 2026.
```

- Invalidated facts keep their file, with `invalid_at` filled in.
- `~/.neomem/` is a **local git repository**, giving a full history of every change. It never gets a remote, because it holds sensitive data.
- `~/.neomem/` is the only thing to back up (`index.db` and `models/` can be skipped). Everything else can be rebuilt from it.

## 5. Write path (worker)

A cheap background **worker**, not an agent, turns new intake into memories. It makes one model call per batch with structured output, following Honcho's "deriver" pattern.

1. **Intake** → the record gets an `int_` ID and is copied into the intake log, and the job is queued. The call returns immediately.
2. **Batching:**
   - Agent sessions are extracted after **5 minutes of silence** in that session (the timer resets on each new message), and immediately on `PreCompact` or `SessionEnd`.
   - Other intake is processed as its batch fills up or when its timer runs out.
3. **Extraction** (one model call per batch). The prompt contains:
   - the role ("memory extractor")
   - the new records
   - earlier turns of the same session, to resolve "it" and "he"
   - related existing facts
   - each record's `occurred` time and today's date, to resolve "next Thursday"

   It returns:
   - **Facts:** explicit statements and observed states only (§3.2), each with its time range.
   - **Entities:** mentioned entities, matched against existing ones.
4. **Check against existing facts**, for each new fact:
   1. Search for the 5 closest existing facts.
   2. Ask the `checker` model (§13.4; Jev by default) one typed question, a `choice` among **same / extends / updates / contradicts / new**, with a probability (~120–250 ms). Below 0.6 counts as unclear (step 5).
   3. **same** → attach the new intake record as another source and count it as **evidence** (§8). No duplicate is created.
   4. **extends / updates / contradicts** → write the new fact with that link (for `updates`, set the old fact's `invalid_at`, and point `latest_id` of every fact in the old chain at the new fact).
   5. **unclear** → write it as new. The reconciler checks it again at consolidation.
5. **Write** through the single writer (§9.4): Markdown file first, then `state.db`, then `index.db`, then the in-memory indexes.
6. **Embeddings** are computed locally for every fact and entity, with the active embedding model (§13.4).

### 5.1 What becomes memory from an agent session
- **Turned into memories (extracted):** user messages, assistant replies, and a compact record of each action (command run, file changed, test passed or failed, error).
- **Kept only in the intake log and the original transcript:** raw tool output (file contents, build logs).
- Each turn is one intake record, so a fact always points back to the exact turn it came from.

## 6. Consolidation (agents)

Consolidation is where the judgment-heavy cleanup happens, done by tool-using **agents** in batches. This follows the shape of Honcho's dreamer orchestrator: a plain function runs specialists **one after another**, so each sees the previous one's output, and one specialist failing doesn't stop the next.

### 6.1 Specialists, in order

| # | Specialist | Job | Write tools | Tool-round cap |
|---|---|---|---|---|
| 1 | **Fact reconciler** | Settles conflicts (newest wins by default; when unsure, record `contradicts`), handles updates, merges duplicate facts, re-checks facts whose intake record was corrected or that late data touched | `link(updates/extends/contradicts/part_of)`, `invalidate(fact, at)`, `merge_facts` | 12 |
| 2 | **Entity keeper** | Creates entities that keep coming up, merges duplicates, maintains aliases, records `possibly_same`, adds `part_of` between entities | `create_entity`, `merge_entities`, `add_alias`, `link_entity` | 8 |

Both get read tools: search facts, get recent facts, search the intake log, follow links.

### 6.2 Safeguards (enforced in code, not just requested in prompts)
- **No tool can delete anything.** `invalidate` sets `invalid_at` and adds an `updates` link.
- **No tool can write inferred conclusions.** There is no deduction or induction specialist.
- Every write must cite **real source IDs**. Invented IDs are removed, and a write without valid sources is rejected (copied from Honcho).
- Each specialist only gets the write tools its job needs.
- Merges keep every original: `merge_facts` attaches all sources to the surviving fact and marks the duplicate invalid with an `updates` link; `merge_entities` keeps aliases and `merged_from`, so it can be undone, and old IDs keep resolving.

### 6.3 Triggers
**Any one** of these starts a consolidation:
- 50 new facts since the last run
- the nightly run (03:00)
- late data or a correction touching facts that were already reconciled
- `neomem dream` (manual)

### 6.4 Duplicates
- **Same fact learned twice** (same source sent twice, two sources saying the same thing, or an agent's `neomem add` inside a captured session): the write-time check (§5 step 4) attaches it as another source and counts it as evidence. The reconciler catches whatever slipped through.
- **Same item sent twice:** with no dedupe keys there are two identical intake records, but no duplicate facts.

## 7. Agent session capture (Claude Code, Codex, pi)

### 7.1 The transcript is the master record
Every supported agent writes a JSONL transcript as it goes. neomemd reads each one **from where it last stopped** (positions stored in `state.db`), and a small parser per agent turns entries into messages and action records.

| Agent | Transcript |
|---|---|
| Claude Code | `~/.claude/projects/<cwd-slug>/<session>.jsonl` |
| Codex (0.163+) | `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` |
| pi (`@earendil-works/pi-coding-agent` 1.3+) | `~/.pi/agent/sessions/--<cwd-slug>--/<ts>_<uuid>.jsonl` (created at the first message) |

Transcripts are complete (Codex hooks miss hosted tools like web search), are written as the session runs, and can be replayed to fill in older sessions.

### 7.2 Hooks only signal that something new arrived
- **Claude Code** (`~/.claude/settings.json`) and **Codex** (`~/.codex/hooks.json`, with `[features] hooks = true`): background hooks (`"async": true`) on `UserPromptSubmit`, `PostToolUse`, `Stop`, `PreCompact` and `SessionEnd` run `neomem hook cc|codex <event>`, which sends a signal to the daemon and returns instantly (§13.6). Claude Code's `Stop` payload includes `last_assistant_message`, which covers a transcript that hasn't been written yet.
- **pi and the custom pi harness:** a small TypeScript extension (`~/.pi/agent/extensions/neomem.ts`, also loadable with `--extension` or via the SDK) does the same on `turn_end`, `agent_end` and `session_shutdown`, over one persistent socket connection to the daemon (§13.6). It doesn't wait for the call to finish, because pi waits on its handlers.
- **Folder watch and safety sweep:** neomemd watches all three session folders for changes, and every 60 seconds checks them again. This catches missed hooks, crashes and headless `codex exec` runs.
- **Secrets are removed** before anything is written to the intake log.
- Recall mode (§10.3), when switched on, adds separate blocking hooks that read memory back. The capture hooks above never do.

### 7.3 Timing
Extraction starts after 5 minutes of silence in a session (the timer resets on each new message), and immediately on `PreCompact` or `SessionEnd`, so nothing is lost when the context is compacted.

### 7.4 Automation sessions
Sessions that run automations (digests) are **captured in full** like any other session. Facts they write with `neomem add` and facts extracted from the session itself merge through the write-time check (§6.4).

## 8. Decay (facts only)

Decay **only lowers a fact's ranking**. It never archives, hides, expires or deletes anything. Entities don't decay, and there are no expiry dates.

```
score = relevance × D × (1 + 0.5 × importance_norm) × (invalid ? 0.2 : 1)
```

- **relevance:** the vector similarity to the query, or a linked memory's usefulness score (§10.2).
- **D:** activation over **evidence events**, using ACT-R's power law: `B = ln Σ t_j^(−0.5)`, where `t_j` is the time since each evidence event. B is then squashed into **[0.3, 1.5]**.
  - The **0.3 floor** means a strong match is never buried, however old.
  - The power law gives a long tail instead of effectively deleting old facts.
- **Evidence events:**
  - the fact was written
  - the same fact was stated or seen again (a source attached in §5 step 4)
  - an agent explicitly marked it `useful`
  - **Retrievals do not count.** Counting them would make whatever already shows up keep showing up.
- **importance_norm:** importance (1–10, set by the model at extraction) scaled to 0–1. Higher importance also slows decay.
- **Per-kind rates:** stable facts and identity ("my sister is Maya") **don't decay** (D = 1). Preferences decay slowly. Other facts decay at the base rate.
- **Decay is off** for history-style queries: `history`, `raw`, and `search --include-invalid`.

## 9. Storage

No database services. Everything is SQLite files plus indexes held in the daemon's memory.

### 9.1 `index.db` (SQLite): the search index, rebuildable
- One row per fact and entity:
  - content, kind and tags
  - every timestamp, as whole-number milliseconds
  - importance and decay inputs
  - `latest_id`: the newest fact in this fact's `updates` chain (itself if it is the newest), kept current at write time
- **Vectors:** one row per (memory ID, embedding model ID). During a model swap both models' vectors exist side by side (§13.4).
- **Links:** one row per link (§3.5) and per open-labeled entity relation, with `valid_at` / `invalid_at` / `recorded_at`. Links to intake records and to external references are rows here too.
- **Names:** entity names and aliases, for `resolve` and entity filters.
- It can be deleted at any time; `neomem rebuild` re-creates it from the files and `state.db`.

### 9.2 In memory (inside `neomemd`): what search actually reads
Loaded from `index.db` when the daemon starts, and updated by the writer after every write batch:
- **Vector index:** HNSW (`usearch`, 16-bit floats) for the active embedding model, with tag, entity and time filters applied during the search. A very selective filter (e.g. one entity with 30 facts) is checked by scanning just those facts instead.
- **Link map:** every link, by memory ID, for the one-hop step (§10.2 step 4).
- **Latest map:** `latest_id` for every fact.
- **Name map:** names and aliases → entity ID.
- Search reads an **immutable snapshot**; the writer builds the next one and swaps it in atomically, so a search never waits for a write and never sees half a write.
- Size: about 150 MB per 100K facts at 768 dimensions (less if a smaller embedding size is chosen).

### 9.3 SQLite (`~/.neomem/state.db`): intake log and state
Everything that **can't** be rebuilt from the Markdown files lives here:
- the **intake log**: an exact, permanent copy of every intake record, with FTS5 keyword search and a date index for `neomem raw`
- **correction records** for intake records
- the **job queue** (jobs are claimed with a lease, so a crash never loses one)
- **transcript read positions** for Claude Code, Codex and pi
- the **consolidation run log**: when each run happened and what it changed
- the **external reference registry** (§11)
- the **recall log** and the per-session list of injected fact IDs (§10.3)
- the daemon's **error log**, which never includes memory content

### 9.4 Keeping everything in sync
1. **One writer.** Every change goes through a single writer task in the daemon, in this order: Markdown file (atomic write and rename) → `state.db` → `index.db` → new in-memory snapshot. Every step is keyed by memory ID, so repeating it after a crash does no harm.
2. Git commits of `~/.neomem/` are batched (every 30 seconds or 50 changes).
3. Files changed by hand are noticed by the file watcher (the writer ignores its own writes by checksum) and re-indexed.
4. A **nightly drift check** compares the files with `index.db` and fixes any differences.
5. `neomem rebuild` re-creates `index.db` and the in-memory indexes from the files and `state.db`. Upgrades rebuild instead of migrating.

### 9.5 Running
launchd keeps `neomemd` running. Nothing else needs to be installed: SQLite and llama.cpp are compiled into the daemon. Postgres + pgvector or Neo4j can be added later as alternative index adapters (§13.3) if neomem ever needs millions of facts or deep graph queries.

## 10. Search and recall

There are two ways memories reach an agent:
- **Agent-called search** (§10.2): the agent runs `neomem search` when it wants something. Always available.
- **Recall mode** (§10.3): when switched on for an agent or project, a gated pipeline injects a few approved memories before the agent answers.

Nothing else is ever injected: no profile, no always-on summary. Outside recall mode, hooks only capture sessions (§7) and never read memory back.

### 10.1 Interfaces (all call the same functions)
- **`neomem` command-line tool**, every command with `--json`:

| Command | Purpose |
|---|---|
| `neomem search "<q>" [--k 10] [--tags] [--entities] [--from] [--to] [--as-of <time>] [--include-invalid]` | main fact search (§10.2) |
| `neomem recall --agent … [--slice <file>]` | recall mode (§10.3): gate → rewrite → search → approve; prints the memories to inject, or nothing |
| `neomem recall on\|off [--agent] [--project]` | switch recall mode on or off |
| `neomem raw "<q>" [--from] [--to] [--source]` | search the raw intake history by keyword and date; no decay ("what did I say on Tuesday?") |
| `neomem entity <name>` | the entity, its aliases, its facts, linked entities and external references |
| `neomem show <id>` | the full file plus its sources |
| `neomem history <id>` | a fact's full update chain (`updates` / `extends`), no decay |
| `neomem add …` | write an intake record (automations and agents) |
| `neomem useful <id>` | explicit evidence signal (§8) |
| `neomem correct <int_id> --reason …` | mark an intake record as logged incorrectly |
| `neomem resolve "<name>"` | name or alias → entity ID (for other systems, §11) |
| `neomem tags [--new <tag>]` | read the tag registry, or create a tag (duplicate-checked) |
| `neomem link <ext_ref> --to <id> --type <type>` | attach an external reference (§11) |
| `neomem dream` | run consolidation now |
| `neomem rebuild` | rebuild `index.db` and the in-memory indexes from the files |
| `neomem mcp` | MCP server with the same tools (relays to the daemon) |
| `neomem hook <agent> <event>` | used by agent hooks: capture signal, or recall when recall mode is on (§13.6) |
| `neomem install` | sets up Claude Code, Codex, pi and launchd (§13.6) |
| `neomem models [use <role> <model>]` | show or swap the model for a role (§13.4) |
| `neomem reextract [--source] [--from]` | re-run extraction from the intake log with the current extractor |

- **Skill:** `~/.agents/skills/neomem/SKILL.md` teaches agents to use the command-line tool. Codex and pi read `~/.agents/skills/` already; Claude Code gets a symlink in `~/.claude/skills/`.
- **MCP:** `neomem mcp` (stdio, with Streamable HTTP mounted in the daemon). It works on Claude Code, Codex and pi, which now has built-in MCP.
- **Custom pi harness:** registers the same functions directly with `pi.registerTool()`.
- **Local HTTP API:** on neomemd, for scripts and other systems.

### 10.2 The search pipeline (`search`)

A fixed pipeline: **no agent, no model call, no background step.** Target: about 10–30 ms end to end on this machine, well under 100 ms (§13.5).

1. **Embed the query** with the local embedding model.
2. **Vector top-k (seeds).** Nearest neighbours in the in-memory vector index (§9.2), `k` = 10 by default (`--k 5` for tighter results), with any tag, entity and time filters. **Old versions are included at this step**, so a query worded like an old fact still finds its chain.
3. **Swap in the latest version.** Each seed is replaced by its chain's `latest_id` (§9.1), and duplicates are merged. A query about "the Oct 15 ship date" returns "Edge ships Nov 1", with a note that it replaced the older fact.
4. **Look at linked memories (one hop, in-memory link map).** For each seed, fetch what it links to:
   - facts linked by `extends`, `part_of` or `contradicts` (each also swapped for its latest version)
   - the entities it is `about`, with their names and aliases
5. **Keep only the useful ones.** Each linked fact gets a usefulness score with no model call:
   `usefulness = similarity(query, linked fact) × link weight × D`
   - link weights: `contradicts` 1.0 (always shown, flagged), `extends` 0.9, `part_of` 0.8
   - `D` is the decay factor (§8)
   - keep a linked fact only if its usefulness is at least 0.8 × the weakest seed's score, at most 2 per seed
6. **Rank and return.** Seeds are ranked by `similarity × D × (1 + 0.5 × importance_norm)` (§8). Each result carries its kept linked facts and entities underneath it, its sources, and its `valid_at`.

**The latest-fact guarantee:**
- Search only ever returns the newest version of a fact (step 3). An invalidated fact appears only with `--include-invalid`, `--as-of` or `history`.
- `updates` links are written by the worker's check (§5 step 4) before the fact is saved, not left for consolidation, so a change is visible to the next search as soon as that batch is processed.
- If two currently-valid facts conflict and no `updates` link settled it yet (a `contradicts` link, or two near-identical facts about the same entity with different content), **the one with the newer `valid_at` (then `recorded_at`) comes first** and both are flagged `conflict`, so the agent is never silently given the older one.
- `--as-of <time>` answers "what was true then": it uses facts valid at that time instead of the chain heads.

**Hidden by default:** invalidated facts, facts whose only source was corrected, and anything tagged `raw` or `system`. Keyword search is only used by `neomem raw` (§9.3).

The pipeline's numbers (`k`, link weights, keep threshold, per-seed cap) live in one `SearchConfig` value, so evaluation runs (§16) can try variants without code changes.

### 10.3 Recall mode (optional automatic injection)

**Off by default** (decided). Switched on per agent or per project (`neomem recall on|off [--agent cc|codex|pi] [--project <path>]`). It never replaces agent-called search; the agent can always search for itself.

| Step | What | Model | Typical time |
|---|---|---|---|
| 0. **Gate** | "Does this turn need memory?" → yes / no with a probability. On no, stop. | judge: Jev | 0.13–0.29 s measured |
| 1. **Rewrite** | Read the session slice and write 3–5 self-contained queries shaped like the facts being looked for. Queries are streamed, and each one is searched as soon as its line is complete. If the planner hasn't finished within 2.5 s, the queries built without a model are used instead: the new prompt, the latest error line, and names not yet seen in the session | the planner model (default GPT-6.1-Sol, low effort, through the ChatGPT login; any provider, §13.4) | ~1.9–2.3 s measured (first query line after ~1.4 s) |
| 2. **Search** | Run each query through the §10.2 pipeline (top-10, latest version, useful links), then merge and remove duplicates | none | ~0.1 s |
| 3. **Approve** | For each candidate: "would this memory help with what's happening right now?" → keep or drop, all candidates scored in one call | judge: Jev | 0.17–0.25 s measured (15 candidates) |
| 4. **Inject** | At most 5 approved memories, each with its `fct_` ID, as extra context | none | — |

A gated-out turn costs ~0.15–0.3 s; a full recall takes about 2.5–3 s, capped at 3.5 s.

**The session slice** (input to the gate, rewrite and approval):
- the last several user and assistant turns, plus one-line summaries of tool calls (files touched, commands run, errors)
- **never raw tool output** (file contents, build logs)
- the new prompt, or the new error when triggered mid-task
- about 2–4K tokens; secrets removed first

**The judge model** (gate and approval): **Jev** by default.
- **What Jev is:** TypeSafe's first "System One" model. Unstructured input in, **typed decisions out**: the possible answers are declared up front (a yes / no, or a choice among up to 255 options), and every answer comes with a calibrated probability. It answers in one parallel pass instead of writing tokens, in about 70–500 ms. Input costs $0.042 per million tokens and output is free (launch pricing, which TypeSafe says may be subsidised). It's a hosted API in early access, so it **needs a Jev API key**.
- **How it's called:** `POST https://api.typesafe.ai/v1/systemone` with `{model, state, questions}`. The session slice goes in `state`. Each question is a `noul` (yes / no → P(yes)), a `choice` (one of up to 255 named options → a probability for each), or a `score` (an ordered scale). Model `jev-latest`, pinned to a version ID (`jev-1.13.0` when measured) so behaviour doesn't change underneath the thresholds. Limits: 64K tokens per request, 32K per question.
- **Gate:** one `noul`, "would long-term memory help with the new message?"
- **Approval:** one request with one `noul` per candidate memory ("would this help with what's happening right now?"). Jev answers them in parallel, each with its own probability.
- **Leans towards no:** approval keeps a memory at P ≥ 0.7; the gate opens at P ≥ 0.5, because a gate that misses costs a whole recall while approval still filters afterwards. Both are tuned on real data. A wrong memory in context does more harm than a missing one, because the agent can still search. Calibrated probabilities make these thresholds meaningful.
- **Why Jev can't be the planner:** it doesn't generate text, so it can't write search queries. The planner stays a text model (§13.4).
- **Measured from this Mac (2026-10-20, `jev-1.13.0`, 11 calls):** median 169 ms, range 117–292 ms. Results:
  - **Gate:** "broken again, same signing error as last week" 0.74; "draft the email to Sam" 0.63; "looks good, commit it" 0.15; a fully specified rename 0.19.
  - **Approval** (15 memories, signing error in the session): the four signing-related memories scored 0.72–0.87, and unrelated ones (sister's name, dark mode, sleep score, other projects) 0.01–0.08.
  - **Write-time check:** 4 out of 4 correct (updates 0.97, same 1.00, extends 1.00, new 1.00).
  - **Cost:** about 1,060 input tokens for an approval call, i.e. about $0.00004.
- **Fallback:** if Jev is unreachable, slow (over 600 ms) or the key is missing, the local reranker (Qwen3-Reranker-0.6B or bge-reranker-v2-m3 through llama.cpp, ~0.1–0.3 s) takes over for that call.
- **Privacy:** the session slice (secrets removed, never raw tool output) goes to TypeSafe's API. With `judge = local`, nothing leaves the machine.

**What the gate looks for:**
- **Likely yes:** a new task or topic; references to the past ("like last time", "again", "we decided"); a new error; names of people, projects or tools not yet mentioned in the session; the start of a session.
- **Likely no:** follow-ups inside the same task ("run it", "looks good"); confirmations; fully specified code instructions; topics whose memories were already injected this session.

**Rules:**
- Never inject the same fact twice in one session (injected IDs are tracked per session in `state.db`).
- Injected memories follow the same latest-fact guarantee as search (§10.2).
- Every gate decision, rewrite, approval and injection is logged in `state.db` (IDs and scores only, never content).
- **Free training labels:** whenever the agent runs `neomem search` on its own in a turn the gate skipped, the gate was wrong. Those logs are used to tune the gate's threshold, or fine-tune it later, with no hand labelling.

**Where it runs:**
- **Claude Code:** a blocking `UserPromptSubmit` hook (`neomem hook cc prompt`) returns the approved memories as additional context. Optionally a `PostToolUse` hook does the same when a tool call fails, so recall also happens mid-task.
- **Codex:** the matching blocking `UserPromptSubmit` hook; Codex returns `additionalContext` the same way Claude Code does.
- **pi and the custom harness:** the neomem extension asks the daemon in `before_agent_start` and adds the result to the turn's context.
- Every recall call has a hard time limit (3.5 seconds) and **fails open**: on a timeout or error the agent simply gets no memories.
- These recall hooks are separate from the capture hooks (§7.2), which stay async.

## 11. Shared contract (for other systems, e.g. a future episode system)

neomem publishes a contract so another system can link its own objects (episodes, notes, anything) to neomem's entities and facts without copying them. The repo ships it as `CONTRACT.md` plus a JSON Schema that both sides build against.

1. **IDs:** permanent, prefixed IDs: `ent_` (entities), `fct_` (facts), `int_` (intake records), and `ext:<system>/<id>` for external references. IDs are never reused or renumbered. **A merged entity's old ID keeps resolving to the survivor.**
2. **Tags:** neomem owns the single tag registry. Other systems read it (`neomem tags`) and create new tags through neomem, so the duplicate check applies to them too.
3. **Link types:** the fixed list in §3.5 is part of the contract, including `mentions` and the other external link types.
4. **Entity lookup:** `neomem resolve "<name>"` returns the entity ID (aliases included), so another system attaches to the right entity instead of creating a duplicate.
5. **External references, both ways:**
   - The other system stores neomem IDs inside its own objects.
   - It can also register a reference back: `neomem link ext:episodes/epi_123 --to ent_edge --type mentions`. neomem stores **only the reference and its type, never the other system's content**, in `state.db` and as a link row in `index.db`.
   - So `neomem entity Edge` can list "mentioned in 14 items in `episodes`", each with the reference to follow.

**Scope:** neomem defines and serves the contract. **The episode system itself is not part of this project and is not being built now.**

## 12. Models

- **Configurable per source:** Claude or a local model (e.g. an 8-bit or 4-bit model served by Ollama, llama.cpp or mlx_lm.server).
- **Any provider for every cloud role** (§13.4): one provider list, and each role names a provider, a model and an effort level.
- **Default:**
  - **GPT-6.1-Sol, low effort, through the ChatGPT login** (`chatgpt` provider, no API key), for recall-mode query rewriting (the planner, §10.3)
  - Claude Sonnet 5.5 for extraction and the consolidation specialists
  - Jev for the write-time check ("same / extends / updates / contradicts / new") and tag matching
- **Judge and checker: Jev** (§10.3), for typed decisions with calibrated probabilities in ~70–500 ms. A local reranker is the fallback. Like every role, they can be pointed at any other provider.
- **Embeddings are always local** (llama.cpp, §13.4), so they never change underneath you and never need a network connection.
- **Every model is swappable** through `config.toml` or `neomem models use` (§13.4).
- Before the model is locked in, **EmbeddingGemma-300m and Qwen3-Embedding-0.6B are benchmarked** on the user's own data. Every vector records which model made it.

## 13. Implementation (module design)

The design follows **deep modules**: each module hides a lot of behaviour behind a small interface, and a seam (a place where an implementation can be swapped) exists only where at least two real implementations exist. Its goals, in order: fast search, every model swappable, the same behaviour on Claude Code, Codex and pi, and easy to test.

### 13.1 Process model
- **`neomemd`** is the only process that holds memory. launchd keeps it running. It owns the models, the in-memory indexes (§9.2), the single writer (§9.4), the worker, consolidation, the scheduler and the file and transcript watchers.
- **`neomem`** is a **thin client**: a small Rust binary with no async runtime, no models and no database code. It sends one request over the local socket `~/.neomem/run/neomemd.sock` and prints the answer. Startup plus round trip is about 2–5 ms.
- **Every surface speaks the same request protocol:** the command-line tool and hooks over the Unix socket; scripts and other systems over local HTTP (`127.0.0.1`, the same handlers); MCP through `neomem mcp`, which relays to the daemon; pi's extension over one persistent socket connection. There is exactly one implementation of every command, in the daemon.
- **Versions:** client and daemon exchange a protocol version on connect. On a mismatch after an upgrade, the client asks launchd to restart the daemon.
- **When the daemon is down:**
  - capture signals are written to `~/.neomem/spool/` as one small file each (atomic, named by a hash of the event) and drained when the daemon is back
  - `neomem add` is spooled the same way, so nothing is lost
  - reads ask launchd to start the daemon and wait up to 2 seconds; if it's still down, `search` falls back to keyword search over the intake log, with a warning
  - hooks never wait and never fail the agent (§13.6)

### 13.2 Modules (Rust workspace, flat `crates/`)

| Crate | Owns | Its interface (what callers must know) | May depend on |
|---|---|---|---|
| `neomem-core` | domain types (facts, entities, intake records, links, IDs, tags), the typed query and filter model, decay and ranking math, the port traits (§13.3) | plain types and pure functions; no I/O, no async runtime | `serde`, `thiserror` only |
| `neomem-proto` | the wire protocol: one `Request` / `Response` enum covering every command, framing, protocol version | serializable types | `neomem-core` (types only) |
| `neomem-vault` | the Markdown master copy and its local git history | `put(memory)`, `get(id)`, `scan()`, `commit_pending()`. Hides paths, the YAML header, atomic write-and-rename, checksums, ignoring its own writes, batched commits | `neomem-core` |
| `neomem-state` | `state.db` (§9.3) | intake append, `raw` keyword search, corrections, job queue (claim / finish / leases), transcript positions, run and recall logs, external references. Hides WAL mode, the single write connection, the read pool and lease reclaim | `neomem-core` |
| `neomem-index` | `index.db` and the in-memory indexes (§9.1–9.2) | `apply(changes)`, `snapshot()` → `knn(vector, filter, k)`, `latest(id)`, `neighbors(id, link types)`, `resolve(name)`; `rebuild(...)`. Hides HNSW, filtered search, the link and latest maps, snapshot swapping and the two vector sets during a model swap | `neomem-core` |
| `neomem-models` | adapters for every model role (§13.3–13.4) and the model host | `ModelHost::get(role)` returns a ready model. Hides loading, warm-up, idle unloading, hot reload on config change, retries, prompt caching, structured output | `neomem-core` |
| `neomem-capture` | one adapter per agent (Claude Code, Codex, pi) and the session slice for recall | `read_new(file, position)` → intake records + new position; `slice(session)` → the recall slice. Hides each transcript format, pi's branch tree, Codex's missing transcript paths, partial lines, secret removal | `neomem-core` |
| `neomem-engine` | **all behaviour:** the writer, worker (extraction + write-time check), consolidation specialists, search pipeline, recall pipeline, scheduler (debounce, triggers, nightly jobs, re-embedding), decay | `Engine::handle(Request) -> Response` plus `Engine::run()` for the background work. Everything else is internal | `core`, `vault`, `state`, `index` and the port traits; never a concrete model or agent adapter |
| `neomem-testkit` | fakes for every port, recorded transcripts, shared contract tests every adapter must pass | test helpers only | `neomem-core` |
| `neomemd` (binary) | config, wiring the concrete adapters into the engine, transports (Unix socket, HTTP via `axum`, MCP via `rmcp`), launchd | — | everything except `neomem-cli` |
| `neomem-cli` (binary `neomem`) | commands, `hook`, `mcp` relay, `install`, spooling and the fallback | — | `neomem-proto`, plus read-only `neomem-state` for the fallback |
| `neomem-eval` | benchmark adapters and the evaluation runner (§16) | talks to the daemon like any other client, so it measures the real path | `neomem-proto` |
| `neomem-control` (binary `neomemctl`) | the control CLI (§17.2): runs an isolated neomem, drives it, captures proof, reads and checks the feature graph | — | `neomem-proto`, `neomem-testkit` |
| `xtask` | the dependency-rule test, the no-comments check, the feature-graph check, latency benchmarks | — | `cargo_metadata` |

Non-Rust parts, in `integrations/`:
- `agents-plugin/`: one plugin folder for **both** Claude Code (`.claude-plugin/`) and Codex (`.codex-plugin/`), sharing `hooks/hooks.json`, `.mcp.json` and the skill.
- `pi/`: the npm package `neomem-pi`, the in-process TypeScript extension.

The control skill lives in `.agents/skills/neomem-control/`: `SKILL.md`, `feature-graph.yaml` and `fixtures/` (§17).

**Dependency rules** (checked by a test in `xtask`, the rust-analyzer way):
- `neomem-core` depends on nothing inside the workspace.
- Storage, model and capture crates depend only on `neomem-core`.
- `neomem-engine` names port traits, never a concrete model or agent adapter.
- Only `neomemd` wires concrete adapters together.
- `neomem-cli` never pulls in the async runtime, the model runtime or the index.

### 13.3 Seams and adapters
A seam exists only where something really varies. Every port has a fake in `neomem-testkit`, and every adapter passes the same contract tests.

| Port (in `neomem-core`) | Interface | Real adapters | Test fake |
|---|---|---|---|
| `Embedder` | `embed(texts) → vectors`, `model_id()`, `dims()` | llama.cpp (in-process, Metal, any GGUF embedding model); any OpenAI-compatible embeddings endpoint (OpenAI, OpenRouter, Hugging Face, Ollama, LM Studio, TEI) | deterministic hash embedder |
| `Judge` | `decide(context, question, options) → choice + probabilities` and `score(context, candidates) → probabilities` (gate, approval, write-time check, tag matching) | **Jev** (TypeSafe API, the default); llama.cpp reranker (GGUF, the offline fallback); any `/rerank` endpoint in the common Cohere-style format (Hugging Face TEI, vLLM, Jina, Voyage, Cohere); **any `Llm` provider** used as a judge (one structured call scores every candidate; slower, ~1–2 s per call in the cloud) | fixed score table |
| `Llm` | `complete(request) → structured output`, with tool use for consolidation, plus an effort level | `openai` wire format (Chat Completions and Responses): OpenAI, OpenRouter, Hugging Face, Gemini, Groq, Together, DeepSeek, Ollama, LM Studio, vLLM, llama.cpp server, mlx_lm.server; `anthropic` (prompt caching, thinking); `chatgpt` (ChatGPT login, direct to the Codex backend, like pi's and Hermes's `openai-codex`); `codex` (a warm `codex app-server` using the ChatGPT login, no API key); `command` (any CLI: request on stdin, answer on stdout) | scripted responses |
| `AgentAdapter` | `read_new(file, position)`, `slice(session)` | Claude Code, Codex, pi | recorded transcripts |
| `Clock` | `now()` | system clock | settable clock (for decay and debounce tests) |

**Deliberately not seams** (concrete modules, tested with the real thing in a temporary folder or in-memory SQLite):
- `neomem-vault`, `neomem-state` and `neomem-index`. They have one implementation each, and SQLite and files run fine inside tests.
- Postgres + pgvector or Neo4j would become a second `neomem-index` implementation only if they are ever needed. The index interface (§13.2) is where they would go. They are not built now (one implementation means a seam that only exists on paper).
- Transports (socket, HTTP, MCP) are not a seam into the engine either. They all turn their input into the same `Request` and call `Engine::handle`.

**Rules taken from what went wrong elsewhere:**
- **No untyped filters.** Filters are a typed value in `neomem-core`, translated in exactly one place (`neomem-index`). Mem0 rewrites filter translation in 10+ adapters.
- **No wide, shallow adapter families.** Mem0 wraps 27 vector stores; Cognee's store interfaces have 22–23 methods. Here a port has 1–3 methods.
- **No global singletons** (Cognee), **no duplicated sync and async copies of the core** (Mem0), no thousand-line config module (Honcho), **no menu of search modes** (Cognee has 21).
- **Models never see real IDs.** Prompts show short numbers (1, 2, 3…) that the engine maps back, so a model can't invent a fact ID (Mem0's trick). Writes that still cite unknown sources are rejected (§6.2).

### 13.4 Models: roles, configuration and swapping
Providers are listed once in `~/.neomem/config.toml`; each model **role** then names a provider, a model and an effort level. Adding a new service is a config entry, not code, as long as it speaks one of six formats: `openai`, `anthropic`, `chatgpt`, `codex`, `command`, or `jev` (decisions only, for the judge and checker roles).

```toml
[providers]
openai      = { kind = "openai", base_url = "https://api.openai.com/v1", key_env = "OPENAI_API_KEY", api = "responses" }
openrouter  = { kind = "openai", base_url = "https://openrouter.ai/api/v1", key_env = "OPENROUTER_API_KEY" }
huggingface = { kind = "openai", base_url = "https://router.huggingface.co/v1", key_env = "HF_TOKEN" }
anthropic   = { kind = "anthropic", key_env = "ANTHROPIC_API_KEY" }
gemini      = { kind = "openai", base_url = "https://generativelanguage.googleapis.com/v1beta/openai", key_env = "GEMINI_API_KEY" }
ollama      = { kind = "openai", base_url = "http://127.0.0.1:11434/v1" }
jev         = { kind = "jev", key_env = "JEV_API_KEY", model = "jev-1.13.0" } # TypeSafe System One model: typed decisions + probabilities
chatgpt     = { kind = "chatgpt" }                                 # ChatGPT login, direct to the Codex backend (as pi / Hermes "openai-codex"); no key
codex       = { kind = "codex" }                                   # ChatGPT login via a warm `codex app-server`; no key
mycli       = { kind = "command", cmd = ["my-llm-cli", "--json"] } # any CLI: request JSON on stdin, answer on stdout

[models]
embedder     = { runtime = "llama.cpp", file = "embeddinggemma-300m-Q8_0.gguf" }
judge        = { provider = "jev", fallback = { runtime = "llama.cpp", file = "qwen3-reranker-0.6b-Q8_0.gguf" } }  # gate + approval
planner      = { provider = "chatgpt", model = "gpt-6.1-sol", effort = "low", timeout = "2.5s" } # recall query rewrite
extractor    = { provider = "anthropic", model = "claude-sonnet-5-5", effort = "medium" }        # worker extraction
checker      = { provider = "jev" }                                                           # same / extends / updates / contradicts / new, tag matching
consolidator = { provider = "anthropic", model = "claude-sonnet-5-5", effort = "high" }          # specialists
idle_unload  = "15m"

[models.extractor_by_source]   # optional per-source override (§12)
"bank-digest" = { provider = "ollama", model = "qwen3:8b" }
```

- **Keys** are read from the environment or `~/.neomem/secrets.env` (permissions 600). They are never written to logs, `config.toml` or git.
- **Effort** is one setting (`low` / `medium` / `high`), translated per provider: OpenAI's reasoning effort, Anthropic's thinking effort, OpenRouter's `reasoning.effort`. It's ignored by models without reasoning.
- **Features differ by provider and are handled inside the adapter, never by callers.** Structured output uses the provider's JSON schema mode where it exists, and otherwise a JSON-only prompt plus validation and one retry. Prompt caching is used where it exists (Anthropic, OpenAI) and skipped elsewhere.
- **The `chatgpt` kind** uses your ChatGPT subscription with no API key, the same way pi's and Hermes's `openai-codex` provider does: a ChatGPT sign-in (OAuth), then direct HTTP calls to ChatGPT's Codex backend.
  - `neomem login chatgpt` gives neomem **its own sign-in and token** in `~/.neomem/secrets.env`. It never reads or refreshes the tokens of pi, Hermes or Codex, because refreshing one app's token can sign the others out.
  - It's not a published API, so it can change without notice. All knowledge of it stays inside this one adapter; if it breaks, the role switches to another provider in config.
  - It uses your plan's Codex usage limits. Models a ChatGPT account can't use (e.g. `gpt-5.3-codex-spark`) are rejected by the backend.
- **Measured planner speed** (GPT-6.1-Sol, low effort, same prompt, 2026-10-19):

  | Route | Per call |
  |---|---|
  | `chatgpt` (direct) | 1.9–3.1 s, first query line after 1.4–2.4 s |
  | `codex` (warm app server) | 2.5–4 s |
  | `codex exec` per call | ~8.5 s |
  | GPT-6-Luna / GPT-5.6-Luna (direct, for comparison) | 1.1–1.7 s, but they write questions rather than fact-shaped queries |

  `codex` and `command` are fine for background roles. A `command` CLI pays its own start-up on every call.
- `neomem models test <role>` sends a tiny request and prints the latency, so a provider can be checked before it's used.

| Role | Swap with | What happens on swap |
|---|---|---|
| judge, planner, extractor, checker, consolidator | config edit or `neomem models use <role> <model>` | the daemon reloads that role; the next call uses the new model. Old facts are not re-extracted unless asked (`neomem reextract`). |
| embedder | the same | vectors from different models can't be compared, so the daemon **re-embeds every fact and entity in the background** into a second vector set. Search keeps using the old model until the new set is complete, then switches over in one step. The old vectors are dropped after the switch. |

- **Local models** (the embedder, and the judge's fallback reranker) run **in-process with llama.cpp on Metal**. They are loaded when first needed, warmed up, and **unloaded after 15 minutes without a search or recall**. Reloading takes about 0.5–1 s once. Capture and the worker don't need them loaded, except to embed new facts, which load the embedder briefly.
- Every vector, every recall decision and every consolidation run records which model made it.

### 13.5 Speed design

**Search budget** (warm daemon, 100K facts):

| Step | Time |
|---|---|
| `neomem` starts and parses arguments | 2–5 ms |
| Unix-socket request and reply | < 0.5 ms |
| Embed the query (300M model, Metal; repeated queries hit a small cache) | 5–20 ms |
| HNSW top-k with filters | 1–3 ms |
| Latest-version swap, one hop, usefulness scores (all in memory) | < 1 ms |
| Load content for the results from `index.db` | ~1 ms |
| Format and print | ~1 ms |
| **Total** | **~10–30 ms** |
| First search after models were unloaded | + 0.5–1 s, once |

**What keeps it fast:**
- the daemon stays warm: models loaded, indexes in memory, SQLite connections open
- the search path never touches the network or another process
- search reads an immutable snapshot, so extraction and consolidation never slow it down
- the thin client has nothing to load
- writes are batched: one model call per extraction batch, embeddings computed in batches, git commits every 30 s

**Recall-mode budget:** gate ~130–290 ms (Jev, measured); rewrite ~1.9–2.3 s (GPT-6.1-Sol, capped at 2.5 s, falling back to queries built without a model); searches start as each streamed query arrives (~50 ms); approval ~170–250 ms (Jev, measured with 15 candidates in one call). **About 2.5–3 s per recalled turn, hard limit 3.5 s, fails open.** Turns the gate skips cost ~0.15–0.3 s. Optional `speculative_rewrite`: start the planner at the same time as the gate and cancel it if the gate says no. That saves the gate's time on recalled turns, but spends a planner call on every turn. The planner is the slow step; a faster model (e.g. a Luna model, ~1.1–1.7 s) can be set in config.

**To measure before the numbers are fixed (§16.5):** embedder latency on 16–32 token queries, HNSW p99 at the real dimension, `neomem` startup time (with `hyperfine`), Jev latency from this machine for the gate and for approval at 15 candidates, and the local fallback's latency.

### 13.6 Agent integration
All three agents use the same pattern: **hooks only send a short signal; the daemon reads the transcript itself** from its stored position (§7.1).

| | Capture (never blocks) | Recall mode (blocking, when on) | Install |
|---|---|---|---|
| **Claude Code** | async command hooks on `UserPromptSubmit`, `PostToolUse`, `Stop`, `SubagentStop`, `PreCompact`, `SessionEnd` → `neomem hook cc <event>` | `UserPromptSubmit` (optionally a failed `PostToolUse`) → `neomem hook cc prompt`, returns `additionalContext` (max 10,000 characters) | the shared plugin, through a plugin marketplace |
| **Codex** | the same events, async → `neomem hook codex <event>` | `UserPromptSubmit` → `additionalContext`, same format | the same plugin folder (`.codex-plugin/`); newly added hooks may need approval in Codex |
| **pi + custom harness** | in-process extension: `turn_end` / `agent_end` send a signal without waiting | `before_agent_start` awaits recall (3.5 s limit) and adds it to the turn's context | `pi install npm:neomem-pi` |

- **`neomem hook`:** reads the hook's JSON from stdin, sends one signal (agent, session ID, transcript path, event) and exits. It **always exits 0, prints nothing on failure, and never takes longer than its time limit** (50 ms for capture, 3.5 s for recall). If the daemon is down, the signal goes to the spool.
- **The daemon doesn't rely on end-of-session hooks:** Codex gives `SessionEnd` about 1 second, Claude Code 1.5 seconds. The daemon also watches the three session folders for file changes, plus the 60-second sweep (§7.2).
- **Codex's `transcript_path` can be missing:** the daemon then finds the rollout file by session ID.
- **pi sessions are trees:** the pi adapter follows the active branch (`parentId`), not the file order.
- **`neomem install`** detects which agents are installed and then:
  - adds the plugin to Claude Code and Codex
  - installs the pi package
  - links the skill into each agent's skills folder
  - installs the launchd job for `neomemd`
- **MCP** (`neomem mcp`) is registered for all three agents for on-demand search, alongside the command-line tool and the skill.

### 13.7 Testing
- **The interface is the test surface.** Behaviour is tested through `Engine::handle(Request)` with the testkit fakes, a temporary vault and in-memory SQLite. No test reaches into the engine's internals, so internal refactors don't break tests.
- **Contract tests:** every adapter of a port passes the same suite (each embedder, judge, LLM provider and agent adapter).
- **Capture tests:** recorded real transcripts from each agent, including compaction, crashes mid-line, pi branches and Codex sessions with no transcript path.
- **Property tests** for the latest-fact guarantee and "nothing is ever deleted" (§16.5).
- **Latency tests** in `xtask`, failing the build if search p95 goes over budget.
- **Proofs:** every feature in the feature graph has a `neomemctl prove` scenario run against a real, isolated daemon. A feature counts as done only when its proof passes (§17).

### 13.8 Rust stack (pin exact versions)

| Part | Crate |
|---|---|
| Runtime, local HTTP | `tokio`, `axum` (daemon only) |
| MCP server | `rmcp` (official SDK) |
| SQLite (+ FTS5) | `rusqlite` (bundled) |
| Vector index | `usearch` (HNSW, 16-bit floats, filtered search) |
| Snapshot swap | `arc-swap` |
| Local models | `llama-cpp-2` (llama.cpp with Metal): embeddings and reranking |
| Cloud and server models | one small HTTP client (`reqwest`) with two wire formats: `openai` (Chat Completions + Responses, which covers OpenAI, OpenRouter, Hugging Face, Gemini, Groq, Together, Ollama, LM Studio, vLLM) and `anthropic`, plus `chatgpt` (the Codex backend's Responses format with a ChatGPT OAuth token); plus the `codex` adapter (JSON-RPC to `codex app-server` over stdio) and the `command` adapter (child process) |
| Scheduling | `croner` + `tokio` timers; job queue as a table in `state.db` |
| File and transcript watching | `notify` with a debouncer |
| Markdown / YAML | `pulldown-cmark`, `serde-saphyr` (not `serde_yml`, which has a security advisory) |
| Wire protocol | length-prefixed JSON over the Unix socket (`serde_json`) |
| Git | the `git` command, called by the writer in batches |

### 13.9 What is copied, and from where

| From | What |
|---|---|
| **Honcho** | the worker / dreamer split; the orchestrator design (specialists run in order, each with its own tools and a tool-round cap); the rule that every write cites real source IDs; extraction prompt rules (explicit atomic facts, relative dates resolved, no unresolved pronouns); conclusions → source messages provenance; searchable raw message history; the job queue in the database with per-session ordering; a reconciler that repairs index drift |
| **Graphiti** | facts with time ranges (`valid_at` / `invalid_at`); prompts for invalidating contradicted facts; `reference_time` handling; the search pipeline described as one config value |
| **Supermemory** | `updates` / `extends` link semantics and the "latest version" idea; background batching; one small set of verbs that every integration wraps |
| **Mem0** | extraction prompts and context (recent turns, related memories, dates); the check against existing facts at write time; the 0.3 decay floor; short numbers instead of real IDs in prompts |
| **Basic Memory** | the Markdown format with a metadata header; the file ↔ index sync with checksums; the "database is just a rebuildable index" approach |
| **atuin** | the Rust workspace split (domain types apart from everything else); a warm daemon on a Unix socket behind a thin CLI; local fallback when the daemon is down |
| **claude-mem** | hooks that send a signal and exit; a spool of atomic files when the worker is down; per-agent adapters; transcript byte-offset checkpoints |
| **rust-analyzer** | a core with no I/O; dependency rules checked by a test |
| **LangMem** | per-session debounce that resets on each new message |

### 13.10 Build order
1. Embedding benchmark (EmbeddingGemma-300m vs Qwen3-Embedding-0.6B, as GGUF through llama.cpp) on the user's own data.
2. `neomem-core`, `neomem-proto`, `neomem-vault`, `neomem-state`; the daemon and the thin client over the socket; intake (`neomem add`, the intake log, `neomem raw`); spooling.
3. `neomem-index`: `index.db`, the in-memory indexes and snapshots, `neomem rebuild`, the drift check.
4. Worker: extraction plus the check against existing facts; the model host and the `Llm` adapters.
5. Search, with the command-line tool, skill file and MCP.
6. Session capture: the three agent adapters, `neomem hook`, the shared plugin, the pi package, the folder watcher and sweep, `neomem install`.
7. Consolidation specialists and triggers.
8. Decay.
9. Shared contract: `resolve`, `tags`, `link`, `CONTRACT.md`.
10. Recall mode: Jev adapter and local fallback, judge benchmark (Jev vs the local rerankers on labelled turns), gate, rewrite, approval, recall hooks, decision logging.
11. Embedding-model swapping (background re-embed and switch-over).
12. Evaluation harness and benchmark adapters (§16), started alongside step 2 so every later step is measured.

Step 0, before all of these: the control skill: `neomemctl`, the feature graph and the first fixtures (§17). From then on, the build order is the feature graph's dependency order (§17.1), not this list.

## 14. Explicitly not doing

- **No episodic memory**: no episodes, composites or timeline. These belong to a possible separate system linked through §11.
- No deduction or induction, no traits, motives, patterns or predictions.
- No peers or perspectives, and no record of who believes a fact.
- No deleting, archiving or hiding by decay, and no expiry dates.
- No decay on entities.
- No dedupe keys.
- No automatic injection outside recall mode (§10.3): no profile, no always-on summary.
- No agent or model call inside `neomem search` itself (no Honcho-style `chat`). Models are only used by recall mode, around the search.
- No raw high-volume data stored in neomem. It stays in its own systems, and automations write digests.
- No remote for the data git repository.
- No database services (Postgres, Neo4j) for now; SQLite and in-memory indexes only (§9).
- No search work inside hooks: hooks only send signals or ask the daemon (§13.6).
- No storing of external systems' content, only references to it.

## 15. Defaults filled in by the spec (not explicitly decided — confirm or change)

- **Merging facts** (§6.2): duplicates are marked invalid with an `updates` link, and all their sources move to the surviving fact.
- **Rate for "other facts"** (§8): the base ACT-R rate. "Preferences decay slowly" is not yet a number.
- **Batching for non-session intake** (§5 step 2): one batch per source, flushed every 30 minutes or once 20 records build up.
- **`system` tag** (§3.4): added for daemon and run records.
- **Entity creation threshold** (§3.3): at least 2 facts referring to it.
- **`neomem useful` command** (§10.1): the explicit evidence signal for decay.
- **Agent entities** (§3.3): identified as product + owner. This was proposed but never explicitly confirmed.
- **Default models** (§12): the planner is GPT-6.1-Sol on low effort (decided). Sonnet 5.5 and Haiku 5.5 for the other roles are not confirmed; any role can point at any provider (decided).
- **Build-method numbers** (§17): at most 3 parallel builders on this machine; gardener after every merge and nightly; `clippy` pedantic. (Decided: separate `neomemctl`, no comments at all, live feature-graph page plus CLI.)
- **Jev numbers** (§10.3, §5): gate opens at 0.5, approval keeps at 0.7, the write-time check treats < 0.6 as unclear, and Jev calls over 600 ms fall back to the local reranker. Speed was measured from this Mac (median 169 ms); the thresholds come from a small first test and need tuning on real data.
- **Planner time limit** (§10.3): 2.5 s, then queries built without a model; recall hard limit 3.5 s. Based on GPT-6.1-Sol measuring 1.9–3.1 s through the ChatGPT login.
- **Local HTTP API** (§10.1): proposed alongside the command-line tool; not explicitly confirmed.
- **Module-design numbers** (§13): git commits every 30 s or 50 changes; hook time limits of 50 ms (capture) and 3.5 s (recall); daemon start wait of 2 s; approval capped at 15 candidates; 16-bit HNSW vectors. (Decided: thin client + daemon, SQLite + in-memory indexes, llama.cpp in-process, 15-minute idle unload, background re-embed, internal crates only.)
- **Recall numbers** (§10.3): 3–5 rewritten queries, at most 5 injected memories, a 2–4K-token session slice, mid-task recall only on failed tool calls. These need tuning on real data.
- **Search numbers** (§10.2): `k` = 10, link weights 1.0 / 0.9 / 0.8, keep threshold 0.8 × the weakest seed, at most 2 linked facts per seed. These need tuning on real data.
- **Seeds are vector-only** (§10.2), as requested. A query that hinges on an exact name, ID or number can miss when its meaning is vague; the fallback is filtering with `--entities`.
- **`part_of` kept** (§3.5) for entity → entity and fact → fact, now that episodes are gone.
- **⚠ Git vs. sensitive data:** the requirements doc says sensitive data must never end up "in logs, git, or anywhere it doesn't belong", but the confirmed file layout makes `~/.neomem/` a git repository. The spec resolves this as **a local-only repository with no remote**. The alternative is no git at all, relying on `updated_at` plus the run log for history.

## 16. Evaluation (what neomem is tested on)

**Rule: run every reputable memory benchmark that can be downloaded, plus neomem's own tests.** Where a vendor's open harness exists, use it, so neomem's numbers sit next to theirs under the same reader, judge and settings.

### 16.1 Two scores for every benchmark
- **Retrieval score (cheap, no answering model):** feed the benchmark's history in through `neomem add`, let the worker and consolidation run, then check whether `neomem search` returns the evidence the benchmark marks as correct (recall@5 and @10, precision@5). This also serves as the embedding-model benchmark (§12).
- **End-to-end score:** a fixed reader agent answers each question using **only** `neomem search` (and, as a separate arm, recall mode), and a fixed judge grades the answers. Report **accuracy / latency / context tokens** (Supermemory's MemScore triple).

### 16.2 Arms (what every result is compared against)
- **memory-off**: the same agent with no memory
- **full-context**: the whole history in the prompt, where it fits
- **neomem search only** (the default)
- **neomem + recall mode** (§10.3)
- **other systems through the same harness** where adapters exist: Mem0 (open source), Supermemory, Zep, Honcho (self-hosted)

### 16.3 Chat and long-term memory benchmarks

| Benchmark | What it tests | neomem promise it checks | Harness / source |
|---|---|---|---|
| **LoCoMo** | QA over 10 very long conversations (single-hop, multi-hop, temporal, open-domain) | basic recall, temporal | MemoryBench (Supermemory) · `snap-research/locomo` (CC BY-NC) |
| **LongMemEval-S** | 500 questions: knowledge updates, temporal, multi-session, single-session, preferences, abstention | **latest fact** (knowledge-update), `valid_at` / `--as-of` (temporal), abstention | MemoryBench · `xiaowu0162/longmemeval` |
| **LongMemEval-M** | same questions, ~1.5M-token histories | search quality at scale | official repo |
| **BEAM** 100K / 500K / 1M / 10M | memory over very long conversations | scale, knowledge updates, event ordering | Mem0 and Honcho benchmark repos · `Mohammadta/BEAM-10M` |
| **ConvoMem** | 75K QA: user facts, preferences, **temporal changes**, abstention, implicit connections | latest fact, abstention | MemoryBench · `Salesforce/ConvoMem` |
| **MemoryAgentBench** | accurate retrieval, test-time learning, long-range understanding, **conflict resolution (FactConsolidation)** | **latest fact**, incremental intake | `ai-hyz/MemoryAgentBench` |
| **GoodAI LTM Benchmark** | one very long conversation with interleaved memory tests (continual learning) | intake over time, recall across distance | `GoodAI/goodai-ltm-benchmark` |
| Stale-fact / evolving-knowledge sets (code mutation, config migration, dependency bumps, API evolution; arXiv 2606.26511) | does memory serve stale facts after things change | **latest fact** in a coding setting | only if the data is released (to be checked) |
| DMR (MemGPT / Zep) | deep memory retrieval over multi-session chat | basic recall | only if the dataset can be obtained (to be checked) |

### 16.4 Coding-agent memory benchmarks

| Benchmark | What it tests | Harness / source |
|---|---|---|
| **agent-memory-bench** | Claude Code tasks whose solution depends on a fact from earlier sessions; graded by running code; each fact in 5 conditions: present, absent, **superseded, contradictory**, adjacent | `GiulioDER/agent-memory-bench` (Apache-2.0); neomem enters as a Claude Code adapter (hooks + MCP), loading the corpus through neomem's own intake |
| **VibeMemBench** | 111 real repo tasks with 3,634 past trajectories; memory-on vs memory-off; published finding: 4 memory systems failed to beat memory-off in 11 of 12 pairings | `AlibabaResearch/DAMO-ConvAI/VibeMemBench` + `Tongyi-ConvAI/VibeMemBench` (Apache-2.0); neomem adapter = `add` + `search`. Needs Docker and 111 multi-GB images: run on a capped image pool or another machine |
| **Own coding test set** | the user's real Claude Code / Codex transcripts replayed into neomem, with 50–100 hand-written questions that have known answers ("what fixed the Edge signing error?"), including changed facts | built in this repo; private, never published |

### 16.5 neomem's own tests
- **Latest-fact guarantee:** chains of 2–10 updates, out-of-order and late data, corrections; search must always return the head, `--as-of` must return the right historical version.
- **Recall mode:** gate accuracy, approval precision, injected-but-unused rate, and how often the agent searched on a turn the gate skipped (§10.3 free labels). Run as the recall arm of every end-to-end benchmark.
- **Latency:** `neomem search` p50 / p95 against the §13.5 budget (~10–30 ms, hard ceiling 100 ms) at 10K, 100K and 1M facts; the first search after models were unloaded; recall mode end to end; hook overhead per agent turn.
- **Invariants:** nothing is ever deleted; every fact has a real source; `neomem rebuild` gives identical search results; the drift check finds injected differences.
- **Capture:** every turn of a recorded Claude Code / Codex / pi session reaches the intake log exactly once, including after crashes and compaction.

### 16.6 Rules for fair results
- Same reader model, judge, `k` and context budget for every system in a comparison; record all versions and settings with each run.
- Report every category separately. Preference and implicit-connection questions reward inference, which neomem deliberately doesn't do: expect lower scores there and don't tune toward them.
- Never tune on the test questions: keep a held-out split for each benchmark.
- Development runs use stratified samples (e.g. 100 LongMemEval questions spread across categories) and Haiku or a local model for extraction; full runs with the default models only at milestones (LongMemEval gives every question its own history, so a full run means tens of millions of extraction tokens).
- Treat vendor numbers as context only; compare against numbers produced on the same harness.

### 16.7 Order
1. LongMemEval-S retrieval score → picks the embedding model (§12).
2. LoCoMo + LongMemEval-S samples, end to end.
3. MemoryAgentBench FactConsolidation + the latest-fact tests.
4. agent-memory-bench and the own coding test set.
5. Full LoCoMo, LongMemEval-S, ConvoMem, MemoryAgentBench, GoodAI LTM.
6. Scale: BEAM 100K → 10M and LongMemEval-M.
7. VibeMemBench.
8. The recall-mode arm on everything above.

## 17. How neomem is built (pstack method)

neomem is built with poteto's **pstack** method, from her talk "how I shipped 2,500 PRs last month" (Lauren Tan, September 2026). The whole method is about **trust**: running many agents in parallel only works when you can trust their output without checking it yourself. That trust comes from four things:
1. **Verification:** a feature map plus a CLI, so an agent can check its own work.
2. **Skills that teach agents to work like real engineers** (pstack's playbooks).
3. **A codebase built for agents:** agents copy whatever patterns they see, so bad patterns must be impossible.
4. **Fixing every correction at the highest level possible.**

### 17.1 The feature graph (feature map + build plan)
`.agents/skills/neomem-control/feature-graph.yaml` lists every feature of neomem. It is poteto's **feature map** (what exists, what it does, how it's reached) and also the **build plan** (what depends on what). Each feature has:

```yaml
- id: recall.gate
  does: decides with Jev whether a turn needs memory; P >= 0.5 opens it
  reached_via: [hook cc prompt, hook codex prompt, pi before_agent_start, neomem recall]
  module: neomem-engine (recall)
  depends_on: [daemon, search, models.judge]
  proof: recall-gate            # the neomemctl scenario that proves it
  status: planned               # planned | building | proven | broken
  last_proof: null              # path of the latest proof bundle
```

**Rules:**
- A feature is **proven** only when `neomemctl prove <id>` passes. Its status changes only through `prove`, never by hand.
- Agents take features whose dependencies are all proven (`neomemctl map --next`), so independent features are built in parallel.
- A check in `xtask` fails the build when they drift apart:
  - every command, hook, MCP tool and HTTP route in the code is on the graph, and every graph entry exists in the code
  - every feature names a proof scenario that exists
- A **gardener** automation (§17.5) keeps the graph accurate after every merge.

**Initial graph** (dependencies in brackets):

| Area | Features |
|---|---|
| Base | `daemon` (socket, handshake, autostart) · `intake` [daemon] · `spool` [intake] · `raw` [intake] · `correct` [intake] · `vault` [daemon] · `index` [vault] · `drift` [index] |
| Models | `models.embed` [daemon] · `models.judge` (Jev + local fallback) [daemon] · `models.llm` (all provider kinds) [daemon] · `models.swap` (background re-embed) [models.embed, index] · `models.idle` (15-minute unload) [models.embed] |
| Write path | `extract` [intake, vault, index, models.llm] · `check` (Jev write-time check) [extract, models.judge] |
| Search | `search` [index, models.embed] · `latest` [search, check] · `links` [search] · `history` (`history`, `--as-of`, `--include-invalid`) [latest] · `entity` (`entity`, `show`, `resolve`) [index] · `decay` [search] |
| Consolidation | `reconcile` [check] · `entities` [reconcile] |
| Agents | `capture.cc` [intake] · `capture.codex` [intake] · `capture.pi` [intake] · `mcp` [search, intake] · `skill` [search] · `install` [capture.cc, capture.codex, capture.pi, mcp, skill] |
| Recall | `recall.gate` [search, models.judge] · `recall.plan` [models.llm] · `recall.approve` [recall.gate, recall.plan] · `recall.hooks` [recall.approve, capture.cc, capture.codex, capture.pi] |
| Contract | `contract` (`tags`, `link`, `CONTRACT.md`) [entity] |
| Evaluation | `eval` (§16) [search, extract] |

### 17.2 The control CLI (`neomemctl`)
A separate dev-only binary. The real `neomem` never contains fixtures or fake models. It always runs an **isolated** neomem: its own temporary `NEOMEM_HOME`, socket and data, never the user's real memory.

| Command | What it does |
|---|---|
| `neomemctl up [--models fake\|real] [--home <dir>]` · `down` · `logs` | start or stop an isolated daemon; `fake` uses the testkit models (no network, no cost) |
| `neomemctl seed <fixture>` | load fixtures: recorded transcripts from each agent, digests, fact chains with updates and corrections |
| `neomemctl set <key>=<value>` | drive settings: model roles, recall on/off, thresholds, clock |
| `neomemctl hook <agent> <event> [--payload <file>]` | simulate a Claude Code, Codex or pi hook exactly as the agent would send it |
| `neomemctl replay <transcript> [--agent]` | feed a real recorded session through capture |
| `neomemctl trace search "<q>"` | run one search and print every pipeline step's time and output |
| `neomemctl check` | invariants: nothing deleted, every fact cites a real source, latest fact always first, `rebuild` gives identical results, no memory content in logs |
| `neomemctl bench latency\|recall` | latency against the §13.5 budget; recall end to end |
| `neomemctl prove <id>\|--changed\|--all` | run a feature's scenario and write a **proof bundle** to `proofs/<id>/<time>/`: `proof.json` (pass/fail, each check, timings, model and code versions), the requests and responses, and the logs. Exit code 0 only if every check passed. |
| `neomemctl map [--next\|--status\|--check\|--publish]` | show the feature graph: what's proven, building, broken and unblocked. `--check` compares it with the code. `--publish` regenerates the live page (§17.6). |

Proof bundles are evidence, not opinions: a feature's claims are checked against real runs. Examples: "search p95 under 30 ms", "every turn captured exactly once", "a query worded like an old fact returns the newest version".

### 17.3 The loop for every feature (pstack playbooks)
1. `neomemctl map --next` picks an unblocked feature.
2. `poteto-mode` routes it to the **feature** playbook (plan, design, delegate, verify). **architect** is used first when the feature adds a module or changes an interface (§13).
3. **The proof scenario is written first**, at the module's interface (TDD), and fails.
4. Build until `cargo test`, the lints and `neomemctl prove <id>` all pass.
5. **interrogate** (adversarial review by several reviewers), then **deslop** on the diff.
6. `prove` marks the feature proven in the graph and links the bundle.
7. **swarm** runs independent unblocked features in parallel, each in its own worktree. At most 3 builders at once on this 24 GB machine.

### 17.4 Codebase rules (the codebase is memory)
**Whenever an agent is corrected, fix it at the highest level that works**, in this order:
1. **Codebase and types:** make the mistake impossible.
2. **Static analysis:** a lint or check.
3. **Rules and skills:** AGENTS.md and the pstack skills.
4. **Style guide:** only as a last resort, since only humans enforce it.

Built in from the start:
- **Bad states are impossible by type:**
  - No store has a delete method.
  - Facts change only through `invalidate` and new links.
  - IDs are distinct types (`FactId`, `EntityId`, `IntakeId`), so they can't be mixed up.
  - Model output can only reference memories by the short numbers in the prompt.
- **No comments, anywhere** (Dune's rule): no `//`, `/* */` or `///` in Rust. A check in `xtask` runs a Rust lexer over every file and fails on any comment. Agents use comments to justify workarounds, and other agents then copy them. Interfaces explain themselves through names and types, with this spec, `CONTRACT.md` and the feature graph as the written record. Prompts live in their own files, not in code.
- **One paved path** for each recurring job (writing through the writer, adding a provider, adding a command, adding a feature), each with a template the agents copy.
- **Lint first:** when a bad pattern turns up, a lint rule against it lands before (or instead of) the cleanup, so it stops spreading.
- Plus the dependency rules (§13.2), `clippy` at pedantic level, `#![forbid(unsafe_code)]` outside the llama.cpp bindings, and `cargo-deny`.

### 17.5 The gardener
An automation that runs after every merge and nightly. It:
- runs `neomemctl map --check` and fixes drift between the graph and the code
- re-proves features whose code changed (`prove --changed`) and marks failures `broken`
- turns repeated review comments into lint rules or AGENTS.md entries
- reports workarounds and patterns that are starting to spread

### 17.6 The live page
The feature graph is published as a live page (a Claude artifact) that can be opened from a phone. It shows:
- every feature as a node in its dependency graph, coloured by status
- what's unblocked
- the latest proof for each feature, with its key numbers

The build session republishes it whenever a feature's status changes (`neomemctl map --publish` generates it).
