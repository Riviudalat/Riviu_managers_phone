#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Generate `docs/agents/README.md` from the headings in `docs/agents/`.

The index is the only way to resolve a `§9.5` citation after `AGENTS.md` was split
(27/08/2026), and the repo carries 200+ such citations. A hand-maintained index would
drift the moment somebody adds an entry and forgets — which is the exact failure this
whole document set was reorganised to stop. So it is generated, and CI checks that the
committed copy matches what this script produces.

    python scripts/build_agents_index.py           # rewrite the index
    python scripts/build_agents_index.py --check    # exit 1 if it is out of date

Reads only the split files, never `AGENTS.md` (a door with no sections) and never the
index itself (it is generated *from* the headings, so counting it would let it vouch
for itself).
"""
from __future__ import annotations

import argparse
import io
import os
import re
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
AGENTS_DIR = os.path.join(ROOT, "docs", "agents")
INDEX = os.path.join(AGENTS_DIR, "README.md")
# LF, deliberately, and `--check` compares with line endings normalised.
#
# The first version wrote CRLF and CI went red on the very first run: the quality job
# runs on `windows-2025`, `actions/checkout` leaves the file at LF, and a byte comparison
# then fails for a reason that has nothing to do with the index being wrong. Git stores
# this file as LF either way (`core.autocrlf` is on locally and there is no `text=auto`
# rule), so LF is also what a diff will show.
NL = "\n"

# A heading that opens a numbered section: `## 3. Kiến trúc`, `## §9.117 …`.
HEADING = re.compile(r"^(#{2,4})\s+§?\s*(\d+(?:\.\d+)*)[.\s—]")

TOPIC_TITLES = {
    "1": "Dự án này là gì",
    "2": "Đọc mục này TRƯỚC KHI sửa bất cứ thứ gì liên quan tới WDA",
    "3": "Kiến trúc",
    "4": "Chạy và test",
    "5": "Trạng thái bình luận",
    "6": "Cách hiệu chỉnh detector",
    "7": "Nguyên tắc khi sửa code này",
    "8": "Unified Agent Runtime",
    "9": "Fleet Android",
    "10": "Mở đường cho thiết bị mới",
}


def sort_key(num: str) -> tuple:
    return tuple(int(part) for part in num.split("."))


def anchor(title: str) -> str:
    """GitHub-flavoured heading anchor, diacritics kept."""
    slug = title.strip().lower().replace("§", "")
    slug = re.sub(r"[^\w\sÀ-ỹ-]", "", slug, flags=re.UNICODE)
    return re.sub(r"\s+", "-", slug.strip())


def collect() -> list[dict]:
    """Every numbered section in the split files, in file order."""
    sections: list[dict] = []
    for dirpath, dirnames, filenames in os.walk(AGENTS_DIR):
        dirnames.sort()
        for name in sorted(filenames):
            if not name.endswith(".md") or name == "README.md":
                continue
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, AGENTS_DIR).replace(os.sep, "/")
            body = io.open(path, encoding="utf-8").read()
            fenced = False
            for line in body.split("\n"):
                if line.lstrip().startswith("```"):
                    fenced = not fenced
                    continue
                if fenced:
                    continue
                match = HEADING.match(line)
                if not match:
                    continue
                title = line.lstrip("#").strip()
                sections.append(
                    {
                        "num": match.group(2),
                        "title": title,
                        "file": rel,
                        "anchor": anchor(title),
                        "diary": rel.startswith("diary/"),
                        "top": "." not in match.group(2),
                    }
                )
    return sections


def line_count(rel: str) -> int:
    return io.open(os.path.join(AGENTS_DIR, rel), encoding="utf-8").read().count("\n")


def render(sections: list[dict]) -> str:
    topics = [s for s in sections if s["top"] and not s["diary"]]
    diary = [s for s in sections if s["diary"]]
    dupes = {n for n, c in Counter(s["num"] for s in diary).items() if c > 1}
    pipe = chr(92) + "|"
    out: list[str] = []
    w = out.append

    w("<!-- Sinh bởi scripts/build_agents_index.py. Đừng sửa tay: CI kiểm bản này -->")
    w("<!-- khớp với đầu ra của script. Thêm mục xong thì chạy lại script. -->")
    w("")
    w("# Chỉ mục tài liệu agent")
    w("")
    w("`AGENTS.md` ở gốc repo là **cửa vào**; nội dung thật nằm ở đây. File này là bản đồ:")
    w("cho một số mục, nó nói mục đó ở file nào.")
    w("")
    w("## Cách phân giải một trích dẫn")
    w("")
    w("Mã nguồn trích tài liệu này hơn **200 chỗ**, phần lớn dưới dạng `AGENTS.md §9.5`.")
    w("Những trích dẫn đó **vẫn đúng**: số mục là **định danh vĩnh viễn**, không đổi khi file")
    w("bị chia. Tra số trong hai bảng dưới để biết nó ở đâu.")
    w("")
    w("**Viết trích dẫn bằng dấu `§`, không bằng chữ “mục”.** Đây không phải thẩm mỹ: trong")
    w("tiếng Việt “mục” vừa nghĩa *section* vừa nghĩa *item*, và tài liệu này dùng cả hai nghĩa")
    w("thật — “bảng resource không có mục 38.3.2” nói về một dòng trong bảng nhãn, không về một")
    w("mục ở đây. Nên một cổng khoá vào chữ đó sinh dương tính giả **vì cấu trúc**, không phải")
    w("vì tình cờ. Dấu `§` thì không nhập nhằng, và đó là thứ cổng đọc.")
    w("")
    w("Trích dẫn **theo số dòng** thì không sống được qua việc chia file — nhưng chúng đã chết")
    w("trước đó rồi, và đó là điều đáng ghi. Cả **sáu** chỗ trong repo đều đã trỏ lệch 29–33")
    w("dòng trước khi ai chạm vào file:")
    w("")
    w("| trích dẫn | ở đâu | nội dung thật ở | lệch |")
    w("|---|---|---|---|")
    w("| `AGENTS.md 691-692` | `screen.rs`, `nurture/mod.rs` ×2 | dòng 721–722, §3.12 | 30 |")
    w("| `AGENTS.md 968-973` | `ios-driver/src/pmd.rs` ×2 | dòng 1001–1003, §3.14 | 33 |")
    w("| `AGENTS.md 1470-1472` | `.gitattributes` | dòng 1499–1502, §3.18.1 | 29 |")
    w("")
    w("Hai cái đầu vẫn rơi trong đúng mục nên còn đọc được; cái thứ ba rơi sang một đoạn nói về")
    w("chuyện khác hẳn (PyInstaller loại IPython) trong khi nó được trích để giải thích việc")
    w("chuẩn hoá CRLF. Cả sáu đã đổi sang số mục vào 27/08/2026. **Tên symbol và số mục sống")
    w("qua refactor; số dòng thì không.**")
    w("")
    w("Hai cổng CI giữ điều này, cả hai trong `apps/desktop/src-tauri/src/lib.rs`:")
    w("`every_agents_section_citation_resolves` (mọi `§x` phải trỏ tới một mục thật) và")
    w("`agents_md_stays_a_door` (`AGENTS.md` phải ở lại ngắn).")
    w("")
    w("## Mục tham chiếu (§1–§10)")
    w("")
    w("Đọc theo thứ tự này nếu mới nhận dự án.")
    w("")
    w("| § | Chủ đề | File | Dòng |")
    w("|---|---|---|---|")
    for s in sorted(topics, key=lambda x: sort_key(x["num"])):
        title = TOPIC_TITLES.get(s["num"], re.sub(r"^\d+\.\s*", "", s["title"]))
        w(
            "| §%s | %s | [`%s`](%s) | %d |"
            % (s["num"], title, s["file"], s["file"], line_count(s["file"]))
        )
    w("")
    w("**§2 là mục phải đọc trước khi sửa bất cứ thứ gì liên quan tới WDA.** Nó là mục duy nhất")
    w("trong tài liệu này mà bỏ qua có thể làm hỏng thiết bị thật.")
    w("")
    w("Mục con của §3 và §8 (§3.12, §14.2, …) nằm trong cùng file với mục cha; tra ở bảng dưới")
    w("nếu một trích dẫn nêu số con.")
    w("")
    w("## Nhật ký §9.x")
    w("")
    w("%d mục, trong %d file dưới `diary/`. **Thứ tự trong file là thứ tự viết, không phải thứ"
      % (len(diary), len({s["file"] for s in diary})))
    w("tự số và cũng không phải thứ tự thời gian** — trong bản gốc §9.43 nằm giữa §9.20 và")
    w("§9.21, và §9.4 nằm sau §9.17. Đó là lý do bảng này sắp theo **số**: để tra được.")
    w("")
    w("Tên file mang khoảng ngày **đã sắp**, nên hai file có thể trùng khoảng — đó là hệ quả")
    w("thật của việc các mục không được viết theo thứ tự, không phải thứ nên che.")
    w("")
    if dupes:
        w("### Số bị dùng hai lần — đọc trước khi tin một trích dẫn")
        w("")
        w("%d số có nhiều hơn một mục, nên một trích dẫn `§9.43` **không đủ để xác định mục nào**:"
          % len(dupes))
        w("")
        for num in sorted(dupes, key=sort_key):
            rows = [s for s in diary if s["num"] == num]
            cont = sum(1 for r in rows if "tiếp" in r["title"])
            note = (
                "%d mục “tiếp” — cùng một chủ đề viết nhiều đợt, đây là **cố ý**" % cont
                if cont
                else "**hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật"
            )
            w("- **§%s** (%d mục): %s" % (num, len(rows), note))
            for r in rows:
                w("  - [%s](%s#%s)" % (r["title"].replace("|", pipe)[:98], r["file"], r["anchor"]))
        w("")
    w("### Bảng tra")
    w("")
    w("| § | Mục | File |")
    w("|---|---|---|")
    for s in sorted(diary, key=lambda x: (sort_key(x["num"]), x["file"], x["anchor"])):
        title = re.sub(r"^§?\s*9\.\d+\s*(?:—\s*)?", "", s["title"].replace("|", pipe))
        flag = " ⚠️" if s["num"] in dupes else ""
        w(
            "| §%s%s | [%s](%s#%s) | `%s` |"
            % (s["num"], flag, title[:104], s["file"], s["anchor"], s["file"])
        )
    w("")
    w("---")
    w("")
    w("Trước 27/08/2026 tất cả những mục trên nằm trong **một** file 10.385 dòng, không mục lục,")
    w("số mục không theo thứ tự file. Nó đã lừa được chính người viết nó nhiều lần trong tuần đó")
    w("— xem §9.120. Việc chia file không đổi một chữ nào của nội dung: phép chia được kiểm bằng")
    w("cách dựng lại và so từng dòng với bản gốc.")
    return NL.join(out) + NL


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; exit 1 if the committed index is out of date",
    )
    args = parser.parse_args()

    sections = collect()
    # A generator that read nothing would render a plausible, empty index.
    if len(sections) < 120:
        print(
            "only %d sections found under docs/agents/; the heading scan is broken"
            % len(sections),
            file=sys.stderr,
        )
        return 1

    rendered = render(sections)
    if args.check:
        current = io.open(INDEX, encoding="utf-8", newline="").read()
        # Compare content, not line endings: a Windows worktree with `core.autocrlf` on
        # holds CRLF while CI holds LF, and neither means the index is stale.
        if current.replace("\r\n", "\n") != rendered.replace("\r\n", "\n"):
            print(
                "docs/agents/README.md is out of date: run "
                "`python scripts/build_agents_index.py`",
                file=sys.stderr,
            )
            return 1
        print("index up to date (%d sections)" % len(sections))
        return 0

    io.open(INDEX, "w", encoding="utf-8", newline="").write(rendered)
    print(
        "wrote %s: %d sections (%d topic, %d diary)"
        % (
            os.path.relpath(INDEX, ROOT),
            len(sections),
            sum(1 for s in sections if not s["diary"]),
            sum(1 for s in sections if s["diary"]),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
