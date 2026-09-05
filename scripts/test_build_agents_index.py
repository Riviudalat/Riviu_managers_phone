from __future__ import annotations

import unittest

from scripts import build_agents_index as index


class AgentsIndexTests(unittest.TestCase):
    def test_diary_local_list_headings_are_not_global_sections(self):
        sections = index.parse_sections(
            "## §9.103 Evidence\n### 1. First observation\n### 2. Second observation\n",
            "diary/example.md",
        )
        self.assertEqual([row["num"] for row in sections], ["9.103"])

    def test_letter_suffix_is_not_backtracked_to_parent_number(self):
        sections = index.parse_sections("### 5.2b Variant\n", "05-topic.md")
        self.assertEqual([row["num"] for row in sections], ["5.2b"])

    def test_runtime_checkpoints_keep_their_own_titles(self):
        sections = index.parse_sections(
            "## 8. Runtime\n### 9. Context comment\n### 10. Campaign checkpoint\n",
            "08-runtime.md",
        )
        rendered = index.render(sections)
        self.assertIn("Context comment", rendered)
        self.assertIn("Campaign checkpoint", rendered)
        self.assertNotIn("Fleet Android", rendered)

    def test_topic_subsections_are_all_linked(self):
        sections = index.parse_sections(
            "## 3. Architecture\n### 3.12 Session order\n#### 3.18.1 Packaging\n",
            "03-architecture.md",
        )
        rendered = index.render(sections)
        for section in sections:
            self.assertIn(section["file"] + "#" + section["anchor"], rendered)

    def test_fenced_and_indented_headings_are_not_sections(self):
        body = "## 3. Architecture\n~~~md\n## 99. Sample\n~~~\n    ## 98. Code\n"
        self.assertEqual([row["num"] for row in index.parse_sections(body, "03-topic.md")], ["3"])

    def test_repeated_titles_get_distinct_anchors(self):
        rows = index.parse_sections("## 9.115 Again\n## 9.115 Again\n", "diary/repeat.md")
        self.assertEqual([row["anchor"] for row in rows], ["9115-again", "9115-again-1"])

    def test_github_anchor_preserves_spaces_left_by_punctuation(self):
        self.assertEqual(index.anchor("§9.150 Draft — result"), "9150-draft--result")

    def test_unknown_section_collision_is_rejected(self):
        rows = index.parse_sections("## 9.200 One\n## 9.200 Two\n", "diary/repeat.md")
        self.assertTrue(any("9.200" in error for error in index.validate_sections(rows)))

    def test_existing_corpus_has_no_unapproved_collisions(self):
        self.assertEqual(index.validate_sections(index.collect()), [])

    def test_repository_scope_excludes_ignored_copies(self):
        paths = index.repository_files()
        self.assertIn("docs/agents/03-kien-truc.md", paths)
        self.assertFalse(any(path.startswith((".superpowers/", ".agents/", "target/")) for path in paths))


if __name__ == "__main__":
    unittest.main()
