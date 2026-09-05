from pathlib import Path
import tempfile
import unittest

from scripts import check_docs


class DocsLinkTests(unittest.TestCase):
    def test_duplicate_headings_and_fences(self):
        body = "# Start\n# Start\n```md\n# Fake\n```\n"
        self.assertEqual(check_docs.headings(body), {"start", "start-1"})

    def test_missing_anchor_is_reported(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "a.md").write_text("# Start\n[bad](a.md#absent)\n", encoding="utf-8")
            errors = check_docs.inspect(root, ["a.md"])
            self.assertEqual(len(errors), 1)
            self.assertIn("missing heading anchor", errors[0])

    def test_local_untracked_copy_cannot_satisfy_a_link(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "a.md").write_text("[copy](copy.md)\n", encoding="utf-8")
            (root / "copy.md").write_text("# Copy\n", encoding="utf-8")
            self.assertIn("not tracked", check_docs.inspect(root, ["a.md"])[0])

    def test_encoded_anchor_and_path(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "a.md").write_text("[item](space%20name.md#hello-world)\n", encoding="utf-8")
            (root / "space name.md").write_text("# Hello: world\n", encoding="utf-8")
            self.assertEqual(check_docs.inspect(root, ["a.md", "space name.md"]), [])

    def test_links_in_code_do_not_count(self):
        self.assertEqual(check_docs.links("```md\n[no](absent.md)\n```\n    [no](absent.md)\n"), [])


if __name__ == "__main__":
    unittest.main()
