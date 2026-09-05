#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Generate the complete agent section index; --check never writes files."""
from __future__ import annotations

import argparse
from collections import Counter
import html
from pathlib import Path
import re
import subprocess
import sys
import unicodedata

ROOT = Path(__file__).resolve().parents[1]
AGENTS_DIR = ROOT / "docs" / "agents"
INDEX = AGENTS_DIR / "README.md"
HEADING = re.compile(r"^(#{2,4})\s+§?\s*(\d+(?:\.\d+)*[a-z]?)(?:\.(?=\s)|(?=\s|$))")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
LEGACY_COLLISIONS = {"9": 2, "10": 2, "9.43": 2, "9.44": 2, "9.45": 2, "9.105": 2, "9.115": 7}


def repository_files() -> list[str]:
    result = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, check=True, capture_output=True)
    return result.stdout.decode("utf-8").rstrip("\0").split("\0")


def sort_key(num: str) -> tuple:
    return tuple((int(re.match(r"\d+", part).group()), re.sub(r"^\d+", "", part)) for part in num.split("."))


def anchor(title: str) -> str:
    """GFM heading slug, including punctuation-created adjacent hyphens."""
    title = html.unescape(re.sub(r"<[^>]+>", "", title)).strip().lower()
    title = re.sub(r"\[([^]]+)\]\([^)]*\)", r"\1", title)
    title = "".join(char for char in title if char in "-_" or not unicodedata.category(char).startswith(("P", "S")))
    return re.sub(r"\s", "-", title)


def parse_sections(body: str, rel: str) -> list[dict]:
    sections: list[dict] = []
    fence = ""
    anchors: Counter[str] = Counter()
    for line in body.splitlines():
        opening = FENCE.match(line)
        if opening:
            marker = opening.group(1)
            if not fence:
                fence = marker
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = ""
            continue
        if fence:
            continue
        heading = re.match(r"^ {0,3}#{1,6}\s+(.+?)(?:\s+#+)?$", line)
        if not heading:
            continue
        title = heading.group(1)
        base = anchor(title)
        suffix = anchors[base]
        anchors[base] += 1
        slug = base if not suffix else f"{base}-{suffix}"
        match = HEADING.match(line)
        if not match:
            continue
        num = match.group(2)
        diary = rel.startswith("diary/")
        # Local numbered observations in an entry are not global section identifiers.
        if diary and not num.startswith("9."):
            continue
        sections.append({"num": num, "title": title, "file": rel, "anchor": slug,
                         "diary": diary, "top": len(match.group(1)) == 2 and "." not in num})
    return sections


def collect() -> list[dict]:
    sections: list[dict] = []
    for name in repository_files():
        if not name.startswith("docs/agents/") or not name.endswith(".md") or name.endswith("/README.md"):
            continue
        path = ROOT / name
        if path.is_file():
            sections.extend(parse_sections(path.read_text(encoding="utf-8"), name.removeprefix("docs/agents/")))
    return sections


def validate_sections(sections: list[dict]) -> list[str]:
    errors = []
    counts = Counter(row["num"] for row in sections)
    for num, count in counts.items():
        if count > 1 and LEGACY_COLLISIONS.get(num) != count:
            errors.append(f"unapproved section collision: {num} ({count} entries)")
    links = Counter((row["file"], row["anchor"]) for row in sections)
    errors.extend(f"duplicate anchor: {path}#{slug}" for (path, slug), count in links.items() if count > 1)
    return errors


def render(sections: list[dict]) -> str:
    topics = [row for row in sections if row["top"] and not row["diary"]]
    children = [row for row in sections if not row["top"] and not row["diary"]]
    diary = [row for row in sections if row["diary"]]
    counts = Counter(row["num"] for row in sections)
    out = [
        "<!-- Sinh bởi scripts/build_agents_index.py. Không sửa tay. -->", "",
        "# Chỉ mục tài liệu agent", "",
        "`AGENTS.md` là cửa vào. [Kho tài liệu](../README.md) chia hướng dẫn hiện tại, lịch sử và bằng chứng.", "",
        "## Đọc trước", "",
        "- [Hướng dẫn tiếp nhận](agent-runbook.md): ranh giới thay đổi, cổng và bàn giao.",
        "- [Vận hành](../operator-guide.md): đầu vào, kết quả và bước tiếp theo của 12 trang.",
        "- [Phát triển](../developer-guide.md): chủ sở hữu, hợp đồng và lệnh kiểm tra.",
        "- Đọc toàn bộ §2 trước mọi thay đổi WDA/iOS. Không suy rộng bằng chứng Android sang iOS.", "",
        "Số mục là định danh vĩnh viễn. Trích bằng `AGENTS.md §x`, không bằng số dòng.",
        "Nhật ký là bản ghi có ngày; mục mới hơn chỉ thay thế các kết luận mà nó nêu rõ.", "",
    ]

    def table(title: str, rows: list[dict]) -> None:
        out.extend([f"## {title}", "", "| § | Nội dung | File |", "|---|---|---|"])
        for row in sorted(rows, key=lambda item: (sort_key(item["num"]), item["file"], item["anchor"])):
            label = row["title"].replace("|", "\\|")
            flag = " *" if counts[row["num"]] > 1 else ""
            out.append(f"| §{row['num']}{flag} | [{label}]({row['file']}#{row['anchor']}) | `{row['file']}` |")
        out.append("")

    table("Mục tham chiếu (§1–§10)", topics)
    table("Mục con và checkpoint kế thừa", children)
    out.extend([
        "## Số mục kế thừa có nhiều chủ sở hữu", "",
        "Dấu `*` yêu cầu đọc tên file và tiêu đề, không chọn mục đầu tiên theo số.",
        "§9/§10 ở file Fleet/Thiết bị mới là mục tham chiếu chính; các checkpoint cùng số trong §8 giữ tên và vị trí lịch sử.",
        "§9.43, §9.44, §9.45 là các mục khác ngày. §9.105 và §9.115 có các phần tiếp cùng chủ đề.",
        "Khi trích các mục này, ghi thêm ngày/tiêu đề và liên kết trực tiếp. Mục mới dùng số mới; không mở rộng danh sách ngoại lệ để che va chạm.", "",
        "## Mới nhất", "",
    ])
    for row in sorted(diary, key=lambda item: sort_key(item["num"]), reverse=True)[:5]:
        out.append(f"- [§{row['num']}: {row['title']}]({row['file']}#{row['anchor']})")
    out.append("")
    table("Nhật ký §9.x", diary)
    out.extend([
        "## Cổng", "", "```powershell",
        "python -m unittest scripts.test_build_agents_index scripts.test_check_docs -v",
        "python scripts/build_agents_index.py --check", "python scripts/check_docs.py",
        "cargo test -p riviu-managers-phone every_agents_section_citation_resolves --lib --locked",
        "cargo test -p riviu-managers-phone agents_md_stays_a_door --lib --locked", "```", "",
        "Bộ quét chỉ đọc file Git theo dõi; bản sao ignored trong `.agents/`, `.superpowers/`, `target/` không được làm chứng cho cây chính.",
        "File agent mới phải được đưa vào Git trước cổng cuối. Chỉ mục kiểm nội dung chuẩn hoá LF, không lấy CRLF làm thay đổi tài liệu.", "",
    ])
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate without writing")
    args = parser.parse_args()
    sections = collect()
    errors = validate_sections(sections)
    if len(sections) < 120:
        errors.append(f"only {len(sections)} sections found; the heading scan is broken")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    rendered = render(sections)
    if args.check:
        if INDEX.read_text(encoding="utf-8") != rendered:
            print("docs/agents/README.md is out of date: run `python scripts/build_agents_index.py`", file=sys.stderr)
            return 1
        print(f"index up to date ({len(sections)} sections)")
    else:
        INDEX.write_text(rendered, encoding="utf-8", newline="\n")
        print(f"wrote docs/agents/README.md ({len(sections)} sections)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
