"""Generate neomem.excalidraw: a hand-drawn walkthrough board of the neomem spec.

Layout follows the Mem0 explainer video: a central title with branches to
sections, yellow-highlighted section headings, brace lists, numbered steps,
and a worked example in each section.
"""
import json
import random
from pathlib import Path

random.seed(7)
ELS = []
SECT = []  # (key, first element index)


def section(key):
    SECT.append((key, len(ELS)))
INK = "#1e1e1e"
DIM = "#868e96"
BLUE = "#1971c2"
RED = "#e03131"
GREEN = "#2f9e44"
VIOLET = "#6741d9"
HL = "#ffec99"  # highlighter yellow


def _base(kind, x, y, w, h, **kw):
    el = {
        "id": f"{kind[:2]}{len(ELS):04d}", "type": kind, "x": x, "y": y, "width": w, "height": h,
        "angle": 0, "strokeColor": kw.get("stroke", INK), "backgroundColor": kw.get("bg", "transparent"),
        "fillStyle": kw.get("fill", "solid"), "strokeWidth": kw.get("sw", 2), "strokeStyle": kw.get("style", "solid"),
        "roughness": kw.get("rough", 1), "opacity": kw.get("opacity", 100), "groupIds": [], "frameId": None,
        "roundness": kw.get("roundness"), "seed": random.randint(1, 2**31), "version": 1,
        "versionNonce": random.randint(1, 2**31), "isDeleted": False, "boundElements": [], "updated": 1,
        "link": None, "locked": False,
    }
    ELS.append(el)
    return el


def text(x, y, s, size=24, color=INK, align="left", family=1):
    lines = s.split("\n")
    w = max(len(l) for l in lines) * size * 0.56
    h = len(lines) * size * 1.25
    if align == "center":
        x -= w / 2
    el = _base("text", x, y, w, h, stroke=color)
    el.update({"text": s, "originalText": s, "fontSize": size, "fontFamily": family, "textAlign": align,
               "verticalAlign": "top", "baseline": int(size * 0.9), "containerId": None, "lineHeight": 1.25,
               "autoResize": True})
    return el


def rect(x, y, w, h, **kw):
    kw.setdefault("roundness", {"type": 3})
    return _base("rectangle", x, y, w, h, **kw)


def ellipse(x, y, w, h, **kw):
    return _base("ellipse", x, y, w, h, **kw)


def line(points, kind="line", **kw):
    x0, y0 = points[0]
    rel = [[px - x0, py - y0] for px, py in points]
    xs = [p[0] for p in rel]
    ys = [p[1] for p in rel]
    el = _base(kind, x0, y0, max(xs) - min(xs), max(ys) - min(ys), **kw)
    el.update({"points": rel, "lastCommittedPoint": None, "startBinding": None, "endBinding": None,
               "startArrowhead": kw.get("start"), "endArrowhead": kw.get("end", "arrow" if kind == "arrow" else None)})
    if kind == "line":
        el["polygon"] = False
    return el


def arrow(x1, y1, x2, y2, bend=0, **kw):
    if bend:
        mx, my = (x1 + x2) / 2, (y1 + y2) / 2
        dx, dy = x2 - x1, y2 - y1
        n = (dx * dx + dy * dy) ** 0.5 or 1
        pts = [(x1, y1), (mx - dy / n * bend, my + dx / n * bend), (x2, y2)]
        el = line(pts, "arrow", **kw)
        el["roundness"] = {"type": 2}
        return el
    return line([(x1, y1), (x2, y2)], "arrow", **kw)


def brace(x, y, h, **kw):
    """A hand-drawn left curly brace whose tip points left, spanning y..y+h at column x."""
    p = [(x + 14, y), (x + 2, y + 12), (x + 2, y + h / 2 - 14), (x - 12, y + h / 2), (x + 2, y + h / 2 + 14),
         (x + 2, y + h - 12), (x + 14, y + h)]
    el = line(p, **kw)
    el["roundness"] = {"type": 2}
    return el


def heading(x, y, s, size=56):
    w = len(s) * size * 0.56
    rect(x - 14, y + size * 0.35, w + 28, size * 0.85, stroke="transparent", bg=HL, rough=0, roundness=None)
    return text(x, y, s, size)


def underline(x, y, w, color=INK):
    return line([(x, y), (x + w * 0.5, y + 3), (x + w, y - 1)], stroke=color, sw=2)


def card(x, y, w, h, title, body, color=INK, bg="transparent", tsize=30, bsize=22):
    rect(x, y, w, h, stroke=color, bg=bg, fill="hachure" if bg != "transparent" else "solid")
    text(x + 20, y + 16, title, tsize, color)
    text(x + 20, y + 16 + tsize * 1.5, body, bsize)


def bullets(x, y, items, size=24, gap=1.45, color=INK):
    for i, it in enumerate(items):
        text(x, y + i * size * gap, "- " + it, size, color)
    return y + len(items) * size * gap


def num(x, y, n, size=26):
    ellipse(x - 6, y - 4, size * 1.35, size * 1.35, stroke=INK)
    text(x + size * 0.18, y, str(n), size)


# Layout grid (top-left of each section). Left column: intro, memory model, ingestion.
# Top middle: stores + models. Bottom middle: consolidation, shared contract.
# Right: search, recall mode, decay.
X0, Y0 = -4000, -2400   # 0. long-term memory
X1, Y1 = 700, -2600     # 1. stores
X7, Y7 = 700, -1100     # models
X2, Y2 = -4000, -900    # 2. memory model
X3, Y3 = -4000, 500     # 3. ingestion
X4, Y4 = -900, 1200     # 4. consolidation
X5, Y5 = 2700, -600     # 5. search
X8, Y8 = 4250, -600     # 6. recall mode
X6, Y6 = 2700, 1300     # 7. decay
X9, Y9 = -900, 2550     # 8. shared contract
X10, Y10 = 2700, -2750  # 9. modules
X11, Y11 = 6100, -2750  # 10. how it is built

section("center")
# ─────────────────────────────── center ───────────────────────────────
ellipse(-330, -150, 660, 300, sw=3)
text(0, -88, "neomem", 104, align="center")
text(0, 48, "semantic memory for any agent", 26, DIM, align="center")

section("ltm")
# ─────────────────────── 0. what long-term memory is ───────────────────────
heading(X0, Y0, "0. What is long-term memory?")
text(X0, Y0 + 110, "stateless", 30)
underline(X0, Y0 + 150, 140)
text(X0, Y0 + 200, "prompt", 28)
arrow(X0 + 120, Y0 + 220, X0 + 240, Y0 + 220)
ellipse(X0 + 250, Y0 + 170, 150, 100)
text(X0 + 325, Y0 + 200, "LLM", 32, align="center")
arrow(X0 + 410, Y0 + 220, X0 + 530, Y0 + 220)
text(X0 + 545, Y0 + 200, "completion", 28)
text(X0 + 545, Y0 + 245, "(remembers nothing)", 20, DIM)

text(X0, Y0 + 330, "stateful (conversational memory)", 30)
underline(X0, Y0 + 370, 470)
text(X0, Y0 + 425, "user\nmessage", 26)
arrow(X0 + 130, Y0 + 455, X0 + 240, Y0 + 455)
ellipse(X0 + 250, Y0 + 400, 170, 110)
text(X0 + 335, Y0 + 435, "Agent", 32, align="center")
arrow(X0 + 430, Y0 + 455, X0 + 540, Y0 + 455)
text(X0 + 555, Y0 + 438, "response", 28)
brace(X0 + 760, Y0 + 400, 220)
text(X0 + 790, Y0 + 410, "history\n  message 1\n  ...\n  message n\n  new message", 22)
text(X0 + 1000, Y0 + 470, "<- lives inside ONE session\n    new chat = forgets", 22, RED)

arrow(X0 + 300, Y0 + 520, X0 + 300, Y0 + 650, start="arrow")
ellipse(X0 + 150, Y0 + 660, 300, 150, sw=3)
text(X0 + 300, Y0 + 705, "Long-term\nMemory", 30, align="center")
text(X0 + 500, Y0 + 650, "<- outside the conversation, survives every session", 24)
text(X0 + 500, Y0 + 695, "<- shared by Claude Code, Codex, pi, your harness", 24)
text(X0 + 500, Y0 + 740, "<- sessions are captured automatically", 24)
text(X0 + 500, Y0 + 785, "<- agents SEARCH it when they need it", 24)

text(X0, Y0 + 900, "neomem's rules", 32, BLUE)
underline(X0, Y0 + 942, 230, BLUE)
bullets(X0, Y0 + 975, [
    "semantic only: facts + entities (no episodes)",
    "nothing is ever deleted",
    "no inference: only what was said or observed",
    "every fact links to where it came from",
    "Markdown files are the master copy",
    "search always returns the LATEST version of a fact",
    "nothing injected unless you switch on recall mode",
    "local, English, quality over speed",
], 24, 1.4)

section("stores")
# ─────────────────────────────── 1. stores ───────────────────────────────
heading(X1, Y1, "1. Stores")
text(X1 + 30, Y1 + 600, "neomem", 34)
brace(X1 + 250, Y1 + 110, 1080, sw=3)

sy = Y1 + 110
text(X1 + 300, sy, "Markdown files  ~/.neomem/", 30, GREEN)
underline(X1 + 300, sy + 42, 420, GREEN)
text(X1 + 760, sy, "<- MASTER COPY", 26, GREEN)
bullets(X1 + 330, sy + 60, ["one file per fact / entity: YAML header + plain text",
                            "facts/2026/10/<id>_...md   entities/<kind>/...",
                            "local git repo (no remote) = full change history"], 22)

sy += 260
text(X1 + 300, sy, "SQLite  index.db", 30, BLUE)
underline(X1 + 300, sy + 42, 250, BLUE)
brace(X1 + 640, sy - 5, 230)
bullets(X1 + 680, sy - 10, ["content, tags, timestamps (epoch ms)", "importance + evidence",
                            "vectors, per embedding model", "every link, with valid_at / invalid_at",
                            "latest_id -> newest version of the fact"], 22, 1.4)
text(X1 + 1260, sy + 60, "REBUILDABLE index\n(can be deleted any time)", 22, DIM)

sy += 290
text(X1 + 300, sy, "in memory (neomemd)", 30, VIOLET)
underline(X1 + 300, sy + 42, 290, VIOLET)
brace(X1 + 640, sy - 5, 160)
bullets(X1 + 680, sy - 10, ["vector index (HNSW) for the active embedding model",
                            "link map (1 hop) · latest map · name + alias map",
                            "search reads a frozen snapshot; the writer swaps in the next one"], 22, 1.5)
text(X1 + 1500, sy + 10, "no Postgres, no Neo4j:\nno DB services, ~2 GB less RAM", 22, RED)

sy += 270
text(X1 + 300, sy, "SQLite  state.db", 30, RED)
underline(X1 + 300, sy + 42, 250, RED)
brace(X1 + 640, sy - 5, 230)
bullets(X1 + 680, sy - 10, ["intake log: exact copy of every input, forever (`neomem raw`)",
                            "job queue · external reference registry",
                            "transcript read positions (cc / codex / pi)",
                            "consolidation + recall logs (ids and scores only)",
                            "error log (never memory content)"], 22, 1.4)
text(X1 + 1420, sy + 60, "= what can NOT be\n   rebuilt from files", 22, RED)

sy += 300
text(X1 + 300, sy, "one writer:  .md file  ->  state.db  ->  index.db  ->  new snapshot   (keyed by id, safe to repeat)", 24)
text(X1 + 300, sy + 40, "nightly drift check   ·   `neomem rebuild` recreates the index from the files", 24, DIM)

section("model")
# ─────────────────────────────── 2. memory model ───────────────────────────────
heading(X2, Y2, "2. What a memory is")
card(X2, Y2 + 110, 760, 430, "INTAKE RECORD   (where from)",
     "- exact copy of every input, forever\n  (a turn, a message, a report)\n- int_ id · occurred start -> end · tags\n"
     "- raw history, NOT a memory\n- NEVER edited: wrong? add a\n  correction record (corrects ->)\n"
     "- every fact: derived_from -> int_\n- searchable: `neomem raw`", BLUE)
card(X2 + 820, Y2 + 110, 760, 430, "FACT   (what is true)",
     "- about >= 1 entity (just who it's about)\n- valid_at / invalid_at = right fact\n  at any point in time\n"
     "- stated or directly observed only\n- updates · extends · contradicts\n- importance 1-10\n"
     "- the ONLY thing that decays\n- future plans live here", GREEN)
card(X2 + 1640, Y2 + 110, 700, 430, "ENTITY   (what it's about)",
     "- person, agent, codebase, place,\n  event, idea ... (open list)\n- emerges when >= 2 facts\n  mention it\n"
     "- aliases + merged_from (undoable;\n  old ids resolve to the survivor)\n- possibly_same -> review\n- agent = product + owner", VIOLET)
text(X2, Y2 + 580, "facts + entities:  recorded_at · updated_at  (+ tags: health, code, money, ... raw, system)", 24, DIM)

ty = Y2 + 700
text(X2, ty, "eg. a fact changing over time", 26, RED)
line([(X2, ty + 120), (X2 + 1800, ty + 120)], "arrow", stroke=INK)
for i, lab in enumerate(["Oct 2", "Oct 15", "Oct 18", "Nov 1"]):
    xx = X2 + 120 + i * 480
    line([(xx, ty + 108), (xx, ty + 132)])
    text(xx, ty + 140, lab, 20, DIM, align="center")
rect(X2 + 60, ty + 190, 720, 70, stroke=DIM, style="dashed")
text(X2 + 80, ty + 205, "F1  Edge ships Oct 15", 24, DIM)
text(X2 + 80, ty + 270, "valid_at Oct 2 · invalid_at Oct 18", 20, RED)
rect(X2 + 1040, ty + 190, 720, 70, stroke=GREEN)
text(X2 + 1060, ty + 205, "F2  Edge ships Nov 1", 24, GREEN)
text(X2 + 1060, ty + 270, "valid_at Oct 18 · invalid_at null", 20, GREEN)
arrow(X2 + 1035, ty + 225, X2 + 790, ty + 225, stroke=GREEN)
text(X2 + 860, ty + 180, "updates", 22, GREEN)
text(X2 + 60, ty + 320, "F1 is kept forever  ·  search returns F2  ·  `--as-of Oct 10` returns F1", 22)

section("ingest")
# ─────────────────────────────── 3. ingestion ───────────────────────────────
heading(X3, Y3, "3. Ingestion")
text(X3 + 400, Y3 + 30, "cheap WORKER  (1 LLM call per batch, not an agent)", 26, DIM)

num(X3, Y3 + 130, 1)
text(X3 + 60, Y3 + 128, "inputs", 30)
brace(X3 + 220, Y3 + 110, 330)
text(X3 + 260, Y3 + 100, "agent sessions", 26, BLUE)
bullets(X3 + 290, Y3 + 140, ["Claude Code · Codex · pi · your pi harness",
                             "transcript .jsonl = the master record",
                             "hooks / pi extension just nudge: `neomem ingest`",
                             "60s sweep catches anything missed",
                             "secrets stripped first"], 22, 1.35)
text(X3 + 260, Y3 + 320, "automations  (your digests: Oura, bank, email, texts...)", 26, BLUE)
text(X3 + 290, Y3 + 360, "neomem add { source, occurred{start,end}, tags, content,\n              entity_hints?, source_ref? }", 22, family=3)

num(X3, Y3 + 470, 2)
text(X3 + 60, Y3 + 468, "copy into intake log (SQLite, id int_...) -> returns instantly", 28)

num(X3, Y3 + 550, 3)
text(X3 + 60, Y3 + 548, "wait for a quiet moment", 28)
bullets(X3 + 90, Y3 + 595, ["session silent 5 min (timer resets on each message)",
                            "or PreCompact / SessionEnd -> right now",
                            "automations: their batch fills or its timer runs out"], 22, 1.35)

num(X3, Y3 + 730, 4)
text(X3 + 60, Y3 + 728, "extract", 28)
text(X3 + 90, Y3 + 790, "messages", 24)
arrow(X3 + 205, Y3 + 805, X3 + 285, Y3 + 805)
ellipse(X3 + 290, Y3 + 770, 120, 75)
text(X3 + 350, Y3 + 790, "LLM", 26, align="center")
arrow(X3 + 415, Y3 + 805, X3 + 495, Y3 + 805)
text(X3 + 505, Y3 + 780, "{ json }", 28, family=3)
brace(X3 + 680, Y3 + 765, 80)
text(X3 + 710, Y3 + 768, "facts (stated / observed only)\nentities", 22)
text(X3 + 90, Y3 + 880, "context in the prompt:", 24)
brace(X3 + 410, Y3 + 870, 210)
bullets(X3 + 440, Y3 + 860, ["role: memory extractor", "the new messages / report",
                             "earlier turns (resolve \"it\", \"he\")", "related existing facts",
                             "occurred time + today (\"next Thursday\" -> a date)"], 22, 1.35)

num(X3, Y3 + 1120, 5)
text(X3 + 60, Y3 + 1118, "check each new fact against memory  (this keeps search on the latest fact)", 28)
text(X3 + 90, Y3 + 1175, "top-5 similar facts", 24)
arrow(X3 + 365, Y3 + 1190, X3 + 440, Y3 + 1190)
ellipse(X3 + 445, Y3 + 1150, 190, 80)
text(X3 + 540, Y3 + 1172, "Jev", 22, align="center")
arrow(X3 + 640, Y3 + 1190, X3 + 720, Y3 + 1190)
brace(X3 + 750, Y3 + 1105, 200)
text(X3 + 780, Y3 + 1100, "same        -> attach source  (+1 evidence)\n"
                         "extends     -> new fact + extends link\n"
                         "updates     -> new fact, old gets invalid_at,\n"
                         "               latest_id moves to the new fact\n"
                         "contradicts -> new fact + contradicts link\n"
                         "unclear     -> new  (reconciler re-checks)", 22, family=3)

num(X3, Y3 + 1380, 6)
text(X3 + 60, Y3 + 1378, "write the .md file -> embed locally -> index.db + in-memory snapshot", 28)

text(X3, Y3 + 1480, "eg.  you, in Claude Code: \"Ship date moved to Nov 1\"", 26, RED)
text(X3 + 60, Y3 + 1525, "-> intake record  int_77  (Oct 18, 14:05, Claude Code turn)\n"
                         "-> fact F2 \"Edge ships Nov 1\"  about [Edge]   updates -> F1   derived_from -> int_77", 22)

section("dream")
# ─────────────────────────────── 4. consolidation ───────────────────────────────
heading(X4, Y4, "4. Consolidation (dreaming)")
text(X4 + 900, Y4 + 30, "AGENTS with tools, in batches  ·  cleanup only", 26, DIM)
text(X4, Y4 + 120, "runs when ANY of:", 26)
bullets(X4 + 30, Y4 + 165, ["50 new facts", "nightly 3:00", "late data or a correction touches old facts",
                            "`neomem dream`"], 22, 1.35)
arrow(X4 + 120, Y4 + 330, X4 + 120, Y4 + 400)
specs = [("1  Fact reconciler", "12 tool rounds", "updates · contradicts (when unsure) · merge duplicates", GREEN),
         ("2  Entity keeper", "8 tool rounds", "new entities · merges · aliases · possibly_same", VIOLET)]
for i, (t, cap, job, col) in enumerate(specs):
    yy = Y4 + 410 + i * 190
    rect(X4, yy, 1100, 140, stroke=col)
    text(X4 + 24, yy + 18, t, 30, col)
    text(X4 + 760, yy + 24, cap, 22, DIM)
    text(X4 + 24, yy + 78, job, 22)
    if i < len(specs) - 1:
        arrow(X4 + 120, yy + 145, X4 + 120, yy + 185)
text(X4, Y4 + 800, "run in order: each sees the last one's output", 22, DIM)
text(X4, Y4 + 860, "safeguards (enforced in code)", 28, RED)
bullets(X4 + 30, Y4 + 905, ["no tool can delete anything", "no tool can infer (no deduction / induction)",
                             "every write must cite real source ids", "invalidate = set invalid_at + updates link"], 22, 1.35)

ex = X4 + 1200
text(ex, Y4 + 410, "eg. reconciler", 26, RED)
for i, lab in enumerate(["F1  Edge ships Oct 15   (valid)", "F2  Edge ships Nov 1    (valid)"]):
    yy = Y4 + 470 + i * 75
    rect(ex, yy, 420, 56, stroke=DIM)
    text(ex + 16, yy + 14, lab, 20, DIM)
    arrow(ex + 425, yy + 28, ex + 560, Y4 + 545, stroke=DIM)
text(ex, Y4 + 625, "slipped through as \"new\" -> conflict", 20, RED)
rect(ex + 565, Y4 + 480, 380, 130, stroke=GREEN, sw=3)
text(ex + 585, Y4 + 495, "F2 updates F1\nF1 invalid_at Oct 18\n(kept forever)", 22, GREEN)
text(ex, Y4 + 720, "eg. entity keeper", 26, RED)
text(ex, Y4 + 775, "\"Sam\" + \"Sam Kim\" + sam@acme.com\n-> ent_sam   (aliases + merged_from kept)", 22, VIOLET)

section("retrieve")
# ─────────────────────────────── 5. search ───────────────────────────────
heading(X5, Y5, "5. Search")
text(X5 + 340, Y5 + 30, "the agent calls it  ·  a fast pipeline, never an agent", 26, DIM)
text(X5, Y5 + 120, "{ q, k=10, tags?, entities?, from?, to?, as_of?, include_invalid? }", 26, family=3)
text(X5, Y5 + 170, "neomem search  ·  SKILL.md for every agent  ·  MCP  ·  pi.registerTool", 22, DIM)
steps = [("embed the query", "local model  ·  no LLM anywhere in search"),
         ("vector top-k  (seeds)", "in-memory HNSW, k = 10 (or 5)  +  tag / entity / time filters  ·  old versions included"),
         ("swap in the LATEST version", "seed -> its chain's latest_id:  a query about \"Oct 15\" returns \"Nov 1\""),
         ("look at linked memories  (link map, 1 hop)", "extends · part_of · contradicts facts  +  the entities it's about"),
         ("keep the useful ones", "similarity(query, fact) x link weight x decay  ·  max 2 per seed"),
         ("rank", "similarity x decay x importance  ·  unsettled conflict: newer first, both flagged"),
         ("results -> agent", "each hit + its linked facts, sources, valid_at  ·  ~10-30 ms")]
for i, (s, d) in enumerate(steps):
    yy = Y5 + 260 + i * 140
    num(X5, yy, i + 1)
    text(X5 + 60, yy - 2, s, 28, GREEN if i == 2 else INK)
    text(X5 + 60, yy + 42, d, 22, DIM)
    if i < len(steps) - 1:
        arrow(X5 + 12, yy + 40, X5 + 12, yy + 120, stroke=DIM)
text(X5, Y5 + 1260, "other tools", 26)
brace(X5 + 180, Y5 + 1245, 190)
text(X5 + 215, Y5 + 1245, "raw \"<q>\"  (intake history: keyword + date, no decay)\n"
                          "entity <name>  (aliases, links, external refs)\n"
                          "show <id>  (file + sources)\nhistory <id>  (update chain, no decay)\n"
                          "recall on|off  (see 6.)\n"
                          "resolve · tags · link   (shared contract)\n"
                          "add · useful · correct · dream · rebuild", 22, family=3)

section("recall")
# ─────────────────────────────── 6. recall mode ───────────────────────────────
heading(X8, Y8, "6. Recall mode")
text(X8 + 500, Y8 + 30, "optional  ·  OFF by default  ·  per agent / project", 26, DIM)
text(X8, Y8 + 120, "before the agent answers (or when a tool call fails), inject a few approved memories", 24)
rsteps = [("0  gate", "does this turn need memory?", "Jev  ~0.2 s", VIOLET),
          ("1  rewrite", "session slice -> 3-5 fact-shaped queries", "GPT-6.1-Sol  ~2 s", BLUE),
          ("2  search", "the 5. pipeline, per query, merged", "no model  ~0.1 s", INK),
          ("3  approve", "would this memory help right now?", "Jev  ~0.2 s", VIOLET),
          ("4  inject", "<= 5 approved facts with ids, never repeated", "no model", GREEN)]
for i, (t, d, m, col) in enumerate(rsteps):
    yy = Y8 + 200 + i * 170
    rect(X8, yy, 800, 120, stroke=col)
    text(X8 + 20, yy + 14, t, 28, col)
    text(X8 + 520, yy + 18, m, 20, DIM)
    text(X8 + 20, yy + 66, d, 22)
    if i < len(rsteps) - 1:
        arrow(X8 + 60, yy + 125, X8 + 60, yy + 165, stroke=DIM)
arrow(X8 + 805, Y8 + 260, X8 + 920, Y8 + 260, stroke=RED)
text(X8 + 930, Y8 + 240, "no -> stop", 22, RED)

rx = X8 + 1150
text(rx, Y8 + 200, "eg. you type:", 26, RED)
text(rx, Y8 + 245, "\"ugh it's broken again, do what we did last time\"", 22)
text(rx, Y8 + 320, "gate:  new error + \"last time\"  ->  YES", 22, VIOLET)
text(rx, Y8 + 380, "rewrite:", 22, BLUE)
text(rx + 30, Y8 + 415, "\"Edge Xcode build failure fix\"\n\"Edge code signing error resolution\"\n\"Edge build was fixed by ...\"", 20, family=3)
text(rx, Y8 + 540, "search:  24 candidates", 22)
text(rx, Y8 + 590, "approve:  2 kept", 22, VIOLET)
text(rx + 30, Y8 + 625, "fct_88  Edge signing fixed by turning on\n        automatic signing in Xcode\nfct_91  Edge CI builds the \"release\" scheme", 20, family=3)
text(rx, Y8 + 760, "inject:  2 facts + ids  ->  agent starts", 22, GREEN)

gy = Y8 + 1080
text(X8, gy, "session slice", 24)
text(X8 + 30, gy + 40, "last turns + 1-line tool summaries, never raw tool output\n~2-4K tokens, secrets removed", 20, DIM)
text(X8, gy + 130, "gate: likely YES", 24, GREEN)
text(X8 + 30, gy + 170, "new task · \"last time\" / \"again\" · new error\nnew names · start of session", 20)
text(X8 + 640, gy + 130, "gate: likely NO", 24, RED)
text(X8 + 670, gy + 170, "\"run it\" · \"looks good\" · confirmations\nfully specified code · already injected", 20)
text(X8 + 1300, gy, "where it runs", 24)
text(X8 + 1330, gy + 40, "Claude Code: UserPromptSubmit (blocking)\n  + PostToolUse when a tool fails\nCodex: same, if its hooks allow (tbd)\npi: extension, before each turn", 20)
text(X8, gy + 270, "Jev leans to NO (calibrated probabilities)  ·  free labels: agent searched on a skipped turn = gate was wrong", 22, RED)

section("decay")
# ─────────────────────────────── 7. decay ───────────────────────────────
heading(X6, Y6, "7. Decay  (facts only)")
text(X6, Y6 + 130, "score = relevance x D x (1 + 0.5·importance) x (invalid ? 0.2 : 1)", 28, family=3)
text(X6, Y6 + 200, "D = ln  sum  t_j ^ -0.5    over EVIDENCE events   -> squashed to [0.3, 1.5]", 24, family=3)
text(X6, Y6 + 260, "evidence:", 26)
bullets(X6 + 30, Y6 + 305, ["the fact was written", "the same fact seen again (source attached)",
                            "an agent ran `neomem useful <id>`"], 22, 1.35)
text(X6, Y6 + 430, "retrievals do NOT count  -> no popularity loop", 24, RED)
cx, cy = X6 + 1150, Y6 + 620
line([(cx, cy), (cx, cy - 360)], "arrow")
line([(cx, cy), (cx + 700, cy)], "arrow")
text(cx - 30, cy - 400, "D", 24, align="center")
text(cx + 720, cy - 14, "time", 22)
pts = [(cx + 10 + k * 34, cy - 60 - 270 / (1 + k * 0.55) ** 0.9) for k in range(20)]
line(pts, stroke=GREEN, sw=3)["roundness"] = {"type": 2}
line([(cx, cy - 95), (cx + 690, cy - 95)], stroke=RED, style="dashed")
text(cx + 470, cy - 135, "floor 0.3", 20, RED)
text(cx + 200, cy - 330, "power law: long tail,\nnever buried", 20, GREEN)
text(X6, Y6 + 520, "never decays:", 26)
bullets(X6 + 30, Y6 + 565, ["entities", "stable facts (\"my sister is Maya\")",
                            "raw / history / --as-of / --include-invalid queries"], 22, 1.35)
text(X6, Y6 + 720, "deleting a memory?  never.", 30, RED)
text(X6, Y6 + 770, "wrong intake record -> correction record · outdated fact -> invalid_at\nduplicate -> invalid + updates link to the survivor", 22)

section("contract")
# ─────────────────────────────── 8. shared contract ───────────────────────────────
heading(X9, Y9, "8. Shared contract")
text(X9 + 620, Y9 + 30, "for a future episode system (not built)", 26, DIM)
cy9 = Y9 + 120
text(X9, cy9, "no episodes in neomem: they can live in ANOTHER system and link in", 26, RED)
rect(X9, cy9 + 70, 560, 190, stroke=DIM, style="dashed")
text(X9 + 24, cy9 + 90, "episode system\n(separate, not built)", 26, DIM)
text(X9 + 24, cy9 + 180, "epi_123 \"work session on Edge\"", 20, DIM)
rect(X9 + 1180, cy9 + 70, 560, 190, stroke=GREEN, sw=3)
text(X9 + 1204, cy9 + 90, "neomem", 30, GREEN)
text(X9 + 1204, cy9 + 145, "ent_edge · fct_... · int_...\nexternal refs listed on the entity", 20)
arrow(X9 + 570, cy9 + 130, X9 + 1170, cy9 + 130, stroke=INK)
text(X9 + 640, cy9 + 90, "stores neomem ids  (via resolve)", 20)
arrow(X9 + 1170, cy9 + 210, X9 + 570, cy9 + 210, stroke=INK)
text(X9 + 640, cy9 + 220, "ext:episodes/epi_123  --mentions-->  ent_edge", 20, family=3)
bullets(X9, cy9 + 310, ["permanent ids: ent_ · fct_ · int_ · ext:   (merged ids resolve to the survivor)",
                        "one tag registry, owned by neomem   (`neomem tags`)",
                        "published link types, incl. mentions   ·   `neomem resolve \"Sam\"` -> ent_sam",
                        "neomem keeps only the reference, never the other system's content",
                        "CONTRACT.md + a JSON Schema in the repo"], 22, 1.35)

section("modules")
# ─────────────────────────────── 9. modules ───────────────────────────────
heading(X10, Y10, "9. Modules")
text(X10 + 430, Y10 + 30, "deep modules: small interface, lots hidden  ·  seams only where something really varies", 26, DIM)
ry = Y10 + 130
rect(X10, ry, 540, 160, stroke=INK)
text(X10 + 24, ry + 20, "Claude Code · Codex · pi\nhooks · skill · MCP · scripts", 24)
arrow(X10 + 550, ry + 80, X10 + 650, ry + 80, stroke=INK)
rect(X10 + 660, ry, 520, 160, stroke=BLUE, sw=3)
text(X10 + 684, ry + 20, "neomem  (thin client)", 26, BLUE)
text(X10 + 684, ry + 70, "2-5 ms  ·  no models, no DB\ndaemon down -> spool, exit 0", 20)
arrow(X10 + 1190, ry + 80, X10 + 1310, ry + 80, stroke=INK)
text(X10 + 1195, ry + 30, "socket", 20, DIM)
rect(X10 + 1320, ry - 20, 1500, 200, stroke=GREEN, sw=3)
text(X10 + 1344, ry, "neomemd  (always running)", 30, GREEN)
text(X10 + 1344, ry + 55, "Engine::handle(Request) -> Response   (one implementation of every command)", 22, family=3)
text(X10 + 1344, ry + 105, "models warm · indexes in memory · ONE writer · worker · consolidation · watchers", 20, DIM)

ly = Y10 + 420
text(X10, ly - 50, "crates  (each row may only use the rows below it)", 24, DIM)
rect(X10, ly, 1000, 90, stroke=INK); text(X10 + 20, ly + 25, "neomemd  — wires adapters · socket · HTTP · MCP", 22)
rect(X10 + 1040, ly, 620, 90, stroke=INK); text(X10 + 1060, ly + 25, "neomem-cli  — no runtime, no models", 22)
rect(X10 + 1700, ly, 600, 90, stroke=INK); text(X10 + 1720, ly + 25, "neomem-eval  — benchmarks", 22)
rect(X10, ly + 150, 2300, 90, stroke=GREEN, sw=3)
text(X10 + 20, ly + 175, "neomem-engine  — ALL behaviour: writer · worker · consolidation · search · recall · scheduler", 22, GREEN)
for i, (n, d) in enumerate([("vault", ".md + git"), ("state", "state.db"), ("index", "index.db + memory"),
                            ("models", "llama.cpp · Claude"), ("capture", "cc · codex · pi")]):
    rect(X10 + i * 465, ly + 300, 440, 90, stroke=INK)
    text(X10 + i * 465 + 20, ly + 312, n + "\n" + d, 20)
rect(X10, ly + 450, 2300, 90, stroke=VIOLET, sw=3)
text(X10 + 20, ly + 475, "neomem-core  — types · typed filters · decay + ranking math · port traits · NO I/O", 22, VIOLET)
for x in (300, 1150, 2000):
    arrow(X10 + x, ly + 95, X10 + x, ly + 145, stroke=DIM)
for x in (220, 1150, 2080):
    arrow(X10 + x, ly + 245, X10 + x, ly + 295, stroke=DIM)
    arrow(X10 + x, ly + 395, X10 + x, ly + 445, stroke=DIM)
text(X10 + 2380, ly, "- neomem-proto: the ONE Request /\n  Response type every surface speaks\n\n"
                    "- engine never names a concrete\n  model or agent adapter\n\n"
                    "- only neomemd wires adapters\n\n- rules checked by a test", 22)

sy10 = Y10 + 1050
text(X10, sy10, "seams  (2+ real adapters each, plus a test fake)", 24)
underline(X10, sy10 + 38, 560)
text(X10, sy10 + 60, "Embedder       llama.cpp (any GGUF) · OpenAI-compatible endpoint\n"
                     "Judge          Jev (TypeSafe API) · llama.cpp reranker (fallback)\n"
                     "Llm            Anthropic · OpenAI-compatible (Ollama, mlx, llama.cpp)\n"
                     "AgentAdapter   Claude Code · Codex · pi\n"
                     "Clock          system · settable fake", 22, family=3)
text(X10 + 1750, sy10, "NOT seams", 24, RED)
text(X10 + 1750, sy10 + 50, "vault · state · index: one real\nimplementation each, tested with\nreal SQLite in a temp folder.\nPostgres / Neo4j = a 2nd index\nadapter, only if ever needed.", 22)

by = Y10 + 1340
text(X10, by, "search:  client 2-5  +  socket <0.5  +  embed 5-20  +  HNSW 1-3  +  latest / 1 hop <1  +  load + print 2   =  ~10-30 ms", 24, GREEN)
text(X10, by + 45, "first search after 15 min idle: +0.5-1 s (models reload)   ·   hooks: capture 50 ms max, recall 3 s max, always fail open", 22, DIM)

section("build")
# ─────────────────────────────── 10. how it's built ───────────────────────────────
heading(X11, Y11, "10. How it's built")
text(X11 + 660, Y11 + 30, "poteto's pstack method  ·  trust = the agent can verify its own work", 26, DIM)
text(X11, Y11 + 120, "\"A feature map gives context. A CLI gives control.\"", 28, VIOLET)
rect(X11, Y11 + 190, 900, 330, stroke=BLUE, sw=3)
text(X11 + 24, Y11 + 210, "feature graph", 30, BLUE)
text(X11 + 24, Y11 + 260, "feature-graph.yaml  =  feature map + build plan", 20, family=3)
bullets(X11 + 24, Y11 + 300, ["~35 features: what it does, how it's reached,", "module, depends_on, proof, status",
                              "agents take features whose deps are proven", "live page + `neomemctl map`"], 20, 1.45)
text(X11 + 940, Y11 + 330, "+", 40, BLUE)
rect(X11 + 1000, Y11 + 190, 900, 330, stroke=GREEN, sw=3)
text(X11 + 1024, Y11 + 210, "neomemctl  (control CLI)", 30, GREEN)
text(X11 + 1024, Y11 + 265, "up --models fake|real   seed <fixture>\nset key=value   hook <agent> <event>\nreplay <transcript>   trace search \"q\"\ncheck   bench   map --next", 20, family=3)
text(X11 + 1024, Y11 + 420, "prove <feature>  ->  proof bundle", 22, RED)
text(X11 + 1024, Y11 + 455, "isolated temp home, never your real memory", 20, DIM)
ly11 = Y11 + 590
text(X11, ly11, "per feature:", 26)
text(X11, ly11 + 45, "map --next -> poteto-mode feature playbook (architect if new interface)\n"
                     "-> write the proof first -> build -> cargo test + lints + prove\n"
                     "-> interrogate + deslop -> prove marks it PROVEN   ·   swarm: max 3 builders", 22)
text(X11, ly11 + 190, "whenever an agent is corrected, fix it at the highest level:", 26)
text(X11, ly11 + 235, "1 codebase + types (impossible)   2 lint / check   3 rules + skills   4 style guide", 22)
text(X11, ly11 + 290, "NO comments at all  ·  no delete method exists  ·  typed IDs  ·  one paved path  ·  gardener after every merge", 22, RED)

section("models")
# ─────────────────────────────── models ───────────────────────────────
text(X7 + 300, Y7 - 30, "models", 30)
underline(X7 + 300, Y7 + 10, 110)
bullets(X7 + 300, Y7 + 40, ["extraction + consolidation: Claude Sonnet, or a local model (per source)",
                            "recall planner: GPT-6.1-Sol low, via your ChatGPT login (~2 s, no key)  ·  Jev writes no text",
                            "ANY provider: ChatGPT login · OpenAI · OpenRouter · Anthropic · Hugging Face · Gemini · Ollama · any CLI",
                            "judge + write-time check: Jev (typed decisions + probabilities, ~0.12-0.29 s measured); local reranker fallback",
                            "embeddings: ALWAYS local (llama.cpp, Metal); EmbeddingGemma-300m vs Qwen3-Embedding-0.6B",
                            "EVERY role swappable in config.toml  ·  new embedder = re-embed in background, then switch",
                            "local models unloaded after 15 min idle"], 22, 1.5)

section("branches")
# ─────────────────────────── branches from center ───────────────────────────
arrow(-330, -40, X0 + 920, Y0 + 40, bend=300)       # 0. what is LTM
arrow(250, -110, X1 - 30, Y1 + 40, bend=80)            # 1. stores
arrow(-340, 30, X2 + 2360, Y2 + 100, bend=40)           # 2. memory model
arrow(-300, 100, X3 + 1300, Y3 + 30, bend=80)          # 3. ingestion
arrow(0, 155, X4 + 500, Y4 - 10, bend=-40)             # 4. consolidation
arrow(335, 20, X5 - 30, Y5 + 40, bend=-60)             # 5. search
arrow(300, -90, X8 + 200, Y8 - 20, bend=-450)          # 6. recall
arrow(280, 110, X6 - 30, Y6 + 40, bend=-120)           # 7. decay
arrow(250, 120, X9 + 1850, Y9 + 40, bend=-1100)         # 8. contract
arrow(330, -60, X10 + 1200, Y10 + 1420, bend=-150)     # 9. modules
arrow(345, -40, X11 + 300, Y11 + 920, bend=-200)      # 10. build

scene = {"type": "excalidraw", "version": 2, "source": "https://excalidraw.com", "elements": ELS,
         "appState": {"viewBackgroundColor": "#ffffff", "gridSize": None}, "files": {}}
out = Path(__file__).with_name("neomem.excalidraw")
bounds = SECT + [("end", len(ELS))]
sections = {k: [e["id"] for e in ELS[a:bounds[i + 1][1]]] for i, (k, a) in enumerate(SECT)}
Path(__file__).with_name("sections.json").write_text(json.dumps(sections))
out.write_text(json.dumps(scene, indent=1))
print(out, len(ELS), "elements")
