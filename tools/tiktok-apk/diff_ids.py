"""Propose old-id -> new-id candidates between two builds' layout indexes.

TikTok renames most obfuscated view ids in every build (``dou`` in 45.7.3 is ``dwh`` in
46.4.3). This tool compares two ``layout_index.py`` outputs and ranks, for each old id,
the new ids that sit in the most similar place:

- layout: same layout name, or a layout whose structure and strings overlap (Jaccard);
- position: same child-index path, else same ancestor tags and depth;
- strings: same resolved text/hint/content-desc (English base of each build);
- local shape: same tag, parent tag, child count and accessibility attributes.

Obfuscated class names (``X.1Ak0``) change between builds and are ignored. The output
is a review table, not a mapping: every row still needs a read on one real phone
before a selector table changes. ``low`` or ``ambiguous`` rows must not be applied.

    py -3.12 -I diff_ids.py --old old\\layout_index.jsonl --new new\\layout_index.jsonl ^
        --ids-file riviu_ids.txt --out candidates.tsv
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

OBFUSCATED_TAG = re.compile(r"^X\.|^[a-zA-Z0-9]{1,4}(\.[a-zA-Z0-9]{1,4})+$")
SEMANTIC_ID = re.compile(r"_|^[a-z][a-zA-Z0-9]{4,}$")
LOCAL_ATTRS = ("visibility", "clickable", "focusable", "importantForAccessibility", "accessibilityLiveRegion")
RARE_SHINGLE = 300
LAYOUT_CANDIDATES = 3
MIN_LAYOUT_SIM = 0.2
TOP = 3
HIGH, HIGH_MARGIN = 0.75, 0.15
MEDIUM, MEDIUM_MARGIN = 0.55, 0.08


def norm_tag(tag: str) -> str:
    return "~" if OBFUSCATED_TAG.search(tag or "") else tag


def strings_of(record: dict) -> frozenset[tuple[str, str]]:
    return frozenset((key, value) for key, value in (record.get("strings") or {}).items() if value)


@dataclass
class Layout:
    key: str
    records: list[dict] = field(default_factory=list)
    shingles: set[str] = field(default_factory=set)


def group(records: list[dict]) -> dict[str, Layout]:
    layouts: dict[str, Layout] = {}
    for record in records:
        key = f"{record['dir']}/{record['layout']}"
        layout = layouts.setdefault(key, Layout(key))
        layout.records.append(record)
        tags = "/".join(norm_tag(tag) for tag in record.get("tags", []))
        layout.shingles.add(f"{record.get('depth')}|{tags}|{norm_tag(record['tag'])}")
        for _, value in strings_of(record):
            layout.shingles.add(f"s|{value}")
    return layouts


def jaccard(a: set[str], b: set[str]) -> float:
    if not a and not b:
        return 0.0
    shared = len(a & b)
    return shared / (len(a) + len(b) - shared)


class LayoutMatcher:
    def __init__(self, new_layouts: dict[str, Layout]):
        self.new = new_layouts
        self.inverted: dict[str, list[str]] = defaultdict(list)
        for key, layout in new_layouts.items():
            for shingle in layout.shingles:
                self.inverted[shingle].append(key)
        self.cache: dict[str, list[tuple[str, float]]] = {}

    def candidates(self, old: Layout) -> list[tuple[str, float]]:
        if old.key in self.cache:
            return self.cache[old.key]
        counts: Counter[str] = Counter()
        for shingle in old.shingles:
            keys = self.inverted.get(shingle, ())
            if len(keys) <= RARE_SHINGLE:
                counts.update(keys)
        pool = {key for key, _ in counts.most_common(20)}
        if old.key in self.new:
            pool.add(old.key)
        scored = sorted(((key, jaccard(old.shingles, self.new[key].shingles)) for key in pool),
                        key=lambda item: (-item[1], item[0]))
        chosen = [item for item in scored[:LAYOUT_CANDIDATES] if item[1] >= MIN_LAYOUT_SIM]
        if old.key in self.new and all(key != old.key for key, _ in chosen):
            chosen.append((old.key, jaccard(old.shingles, self.new[old.key].shingles)))
        self.cache[old.key] = chosen
        return chosen


def element_score(old: dict, new: dict, layout_sim: float) -> tuple[float, str]:
    """Similarity of two id-bearing elements in two candidate layouts, 0..1."""
    old_tags = [norm_tag(tag) for tag in old.get("tags", [])]
    new_tags = [norm_tag(tag) for tag in new.get("tags", [])]
    if old.get("pos") == new.get("pos") and old_tags == new_tags:
        position, why_pos = 1.0, "pos=same"
    elif old_tags == new_tags:
        position, why_pos = 0.6, "pos=same-ancestors"
    elif old.get("depth") == new.get("depth"):
        position, why_pos = 0.3, "pos=same-depth"
    else:
        position, why_pos = 0.0, "pos=differs"

    old_strings, new_strings = strings_of(old), strings_of(new)
    if old_strings:
        if old_strings == new_strings:
            text = 1.0
        elif {v for _, v in old_strings} & {v for _, v in new_strings}:
            text = 0.7
        else:
            text = 0.0
    else:
        text = 0.6 if not new_strings else 0.2
    why_text = "strings=" + ("|".join(sorted(v for _, v in old_strings))[:60] if old_strings else "none")
    why_text += "" if text >= 0.6 else "->" + ("|".join(sorted(v for _, v in new_strings))[:60] or "none")

    local = 0.0
    local += 0.4 if norm_tag(old["tag"]) == norm_tag(new["tag"]) else 0.0
    local += 0.2 if len(old.get("children", [])) == len(new.get("children", [])) else 0.0
    old_parent, new_parent = old.get("parent") or ["", ""], new.get("parent") or ["", ""]
    local += 0.2 if norm_tag(old_parent[0]) == norm_tag(new_parent[0]) else 0.0
    old_attrs, new_attrs = old.get("attrs", {}), new.get("attrs", {})
    local += 0.2 if all(old_attrs.get(key) == new_attrs.get(key) for key in LOCAL_ATTRS) else 0.0

    score = 0.30 * layout_sim + 0.25 * position + 0.30 * text + 0.15 * local
    if old["id"] == new["id"] and SEMANTIC_ID.search(old["id"]):
        score = min(1.0, score + 0.1)
    return score, f"{why_pos}; {why_text}"


@dataclass
class Candidate:
    new_id: str
    score: float
    evidence: str


def propose(old_id: str, old_layouts: dict[str, Layout], matcher: LayoutMatcher) -> list[Candidate]:
    occurrences = [(layout, record) for layout in old_layouts.values()
                   for record in layout.records if record["id"] == old_id]
    if not occurrences:
        return []
    totals: dict[str, float] = defaultdict(float)
    evidence: dict[str, tuple[float, str]] = {}
    for layout, record in occurrences:
        best: dict[str, tuple[float, str]] = {}
        for new_key, sim in matcher.candidates(layout):
            for new_record in matcher.new[new_key].records:
                if not new_record["id"]:
                    continue
                score, why = element_score(record, new_record, sim)
                if score > best.get(new_record["id"], (-1.0, ""))[0]:
                    best[new_record["id"]] = (score, f"{layout.key}->{new_key} sim={sim:.2f}; {why}")
        for new_id, (score, why) in best.items():
            totals[new_id] += score
            if score > evidence.get(new_id, (-1.0, ""))[0]:
                evidence[new_id] = (score, why)
    ranked = sorted(totals.items(), key=lambda item: (-item[1], item[0]))
    return [Candidate(new_id, total / len(occurrences), evidence[new_id][1]) for new_id, total in ranked]


def confidence(candidates: list[Candidate]) -> str:
    if not candidates:
        return "none"
    top = candidates[0].score
    margin = top - (candidates[1].score if len(candidates) > 1 else 0.0)
    if top >= HIGH and margin >= HIGH_MARGIN:
        return "high"
    if top >= MEDIUM and margin >= MEDIUM_MARGIN:
        return "medium"
    return "ambiguous" if margin < 0.05 else "low"


def layout_counts(layouts: dict[str, Layout]) -> dict[str, int]:
    seen: dict[str, set[str]] = defaultdict(set)
    for layout in layouts.values():
        for record in layout.records:
            if record["id"]:
                seen[record["id"]].add(layout.key)
    return {key: len(value) for key, value in seen.items()}


def review_rows(ids: list[str], old_records: list[dict], new_records: list[dict]):
    old_layouts, new_layouts = group(old_records), group(new_records)
    matcher = LayoutMatcher(new_layouts)
    old_count, new_count = layout_counts(old_layouts), layout_counts(new_layouts)
    for old_id in ids:
        candidates = propose(old_id, old_layouts, matcher)
        level = confidence(candidates)
        if not candidates:
            note = "không có trong layout (gán từ code?)" if old_id not in old_count else "không có ứng viên"
            if old_id in new_count:
                note += "; build mới vẫn có id cùng tên trong layout"
            yield [old_id, str(old_count.get(old_id, 0)), "-", "", "", level, "", "", note]
            continue
        for rank, candidate in enumerate(candidates[:TOP], 1):
            yield [old_id, str(old_count.get(old_id, 0)), str(rank), candidate.new_id,
                   f"{candidate.score:.2f}", level if rank == 1 else "",
                   str(new_count.get(candidate.new_id, 0)),
                   "yes" if candidate.new_id == old_id else "", candidate.evidence]


HEADER = ["old_id", "old_layouts", "rank", "new_id", "score", "confidence", "new_layouts", "same_name", "evidence"]


def load(path: Path) -> list[dict]:
    with open(path, encoding="utf-8") as src:
        return [json.loads(line) for line in src if line.strip()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Rank new-build id candidates for old-build ids.")
    parser.add_argument("--old", type=Path, required=True, help="old build layout_index.jsonl")
    parser.add_argument("--new", type=Path, required=True, help="new build layout_index.jsonl")
    parser.add_argument("--ids", nargs="*", default=[], help="old ids to map")
    parser.add_argument("--ids-file", type=Path, help="one old id per line (first TSV column)")
    parser.add_argument("--out", type=Path, help="TSV output (stdout by default)")
    args = parser.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        getattr(stream, "reconfigure", lambda **_: None)(encoding="utf-8")

    ids = list(args.ids)
    if args.ids_file:
        for line in args.ids_file.read_text(encoding="utf-8").splitlines():
            first = line.split("\t")[0].strip()
            if first and not first.startswith("#") and first != "id":
                ids.append(first)
    ids = list(dict.fromkeys(ids))
    if not ids:
        parser.error("pass --ids or --ids-file")
    try:
        old_records, new_records = load(args.old), load(args.new)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    lines = ["\t".join(HEADER)]
    lines += ["\t".join(cell.replace("\t", " ") for cell in row) for row in review_rows(ids, old_records, new_records)]
    text = "\n".join(lines) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8", newline="\n")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
