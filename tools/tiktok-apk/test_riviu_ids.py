"""Riviu id audit on synthetic sources and indexes."""

import json
import tempfile
import unittest
from pathlib import Path

import resources_dump
import riviu_ids

SOURCE = '''\
fn plan(version: &str) -> &'static str {
    match version {
        "45.7.3" => ":id/dou",
        "46.4.3" => "com.zhiliaoapp.musically:id/dwh",
        _ => "com.ss.android.ugc.trill:id/c_4",
    }
}
const SYSTEM: &str = "android:id/button1";
#[cfg(test)]
mod tests {
    const FIXTURE: &str = ":id/fixture_only";
}
'''


class RiviuIdsTests(unittest.TestCase):
    def test_scan_keeps_musically_selectors_and_marks_version_proximity(self):
        usages = riviu_ids.scan([("a.rs", SOURCE)], riviu_ids.PACKAGE, "45.7.3", window=1)
        self.assertEqual(sorted(usages), ["dou", "dwh", "fixture_only"])
        self.assertEqual(usages["dou"].locations, ["a.rs:3"])
        self.assertEqual((usages["dou"].near_version, usages["dwh"].near_version), (1, 1))
        self.assertEqual(usages["fixture_only"].near_version, 0)

    def test_test_code_is_skipped_unless_requested(self):
        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp)
            (repo / "src" / "tests").mkdir(parents=True)
            (repo / "src" / "a.rs").write_text(SOURCE, encoding="utf-8")
            (repo / "src" / "a_tests.rs").write_text('":id/t1"', encoding="utf-8")
            (repo / "src" / "tests" / "b.rs").write_text('":id/t2"', encoding="utf-8")
            production = riviu_ids.scan(riviu_ids.source_files(repo, ["src"], False), riviu_ids.PACKAGE, None, 30)
            everything = riviu_ids.scan(riviu_ids.source_files(repo, ["src"], True), riviu_ids.PACKAGE, None, 30)
        self.assertEqual(sorted(production), ["dou", "dwh"])
        self.assertEqual(sorted(everything), ["dou", "dwh", "fixture_only", "t1", "t2"])

    def test_status_separates_unique_ambiguous_code_only_and_missing(self):
        records = [{"id": "dou", "dir": "layout", "layout": "i74"},
                   {"id": "bix", "dir": "layout", "layout": "a"},
                   {"id": "bix", "dir": "layout-land", "layout": "a"},
                   {"id": "", "dir": "layout", "layout": "a"}]
        dump = "Package name=com.zhiliaoapp.musically id=7f\n  type id id=0a entryCount=1\n    resource 0x7f0a0004 id/s0v\n"
        with tempfile.TemporaryDirectory() as temp:
            index = Path(temp) / "index.jsonl"
            index.write_text("".join(json.dumps(r) + "\n" for r in records), encoding="utf-8")
            layouts = riviu_ids.index_layouts(index)
        table = resources_dump.parse(dump.splitlines(keepends=True))
        status = {name: riviu_ids.classify(name, layouts, table, 0x7F) for name in ("dou", "bix", "s0v", "dmk")}
        self.assertEqual(status, {"dou": "unique", "bix": "ambiguous", "s0v": "code-only", "dmk": "missing"})
        self.assertEqual(riviu_ids.classify("s0v", layouts, None, 0x7F), "not-in-layouts")


if __name__ == "__main__":
    unittest.main()
