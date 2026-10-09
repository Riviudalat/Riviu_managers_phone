"""Old-id -> new-id candidate ranking on synthetic layout indexes."""

import unittest

import diff_ids


def rec(element_id, layout, pos, tag="TextView", strings=None, parent=("LinearLayout", "root"),
        children=(), attrs=None, tags=("FrameLayout", "LinearLayout")):
    return {
        "id": element_id, "layout": layout, "dir": "layout", "tag": tag,
        "depth": len(pos), "pos": list(pos), "tags": list(tags)[:len(pos)],
        "attrs": dict(attrs or {}), "strings": dict(strings or {}),
        "parent": list(parent), "siblings": [], "children": list(children),
    }


def composer(layout, ids, extra=()):
    """A small post bar: container, caption, Post button and a back icon."""
    caption, post, back = ids
    return [
        rec("bar", layout, [0], tag="LinearLayout", tags=("FrameLayout",), children=(caption, post, back)),
        rec(caption, layout, [0, 0], tag="EditText", strings={"hint": "Describe your post"}),
        rec(post, layout, [0, 1], strings={"text": "Post"}, attrs={"clickable": "true"}),
        rec(back, layout, [0, 2], tag="X.1Ak0", strings={"contentDescription": "Back"}),
        *extra,
    ]


def ranked(rows, old_id):
    return [row for row in rows if row[0] == old_id]


class DiffTests(unittest.TestCase):
    def test_renamed_ids_in_a_renamed_layout_map_by_structure_and_strings(self):
        old = composer("uf", ("gpr", "sa3", "bix"))
        new = composer("w2", ("h0a", "t9c", "c1x"))
        for row in new:
            row["tag"] = row["tag"].replace("X.1Ak0", "X.9Zz")  # obfuscated classes change too
        rows = list(diff_ids.review_rows(["gpr", "sa3", "bix"], old, new))
        for old_id, new_id in (("gpr", "h0a"), ("sa3", "t9c"), ("bix", "c1x")):
            first = ranked(rows, old_id)[0]
            self.assertEqual((first[3], first[5]), (new_id, "high"), first)
            self.assertIn("uf->layout/w2", first[8])

    def test_attached_string_outranks_a_shifted_position(self):
        old = [rec("bar", "a", [0], tag="LinearLayout", tags=("FrameLayout",)),
               rec("sa3", "a", [0, 0], strings={"text": "Post"})]
        new = [rec("bar", "a", [0], tag="LinearLayout", tags=("FrameLayout",)),
               rec("k1", "a", [0, 0], strings={"text": "Cancel"}),
               rec("k2", "a", [0, 1], strings={"text": "Post"})]
        first = ranked(diff_ids.review_rows(["sa3"], old, new), "sa3")[0]
        self.assertEqual(first[3], "k2")
        self.assertIn("strings=Post", first[8])

    def test_indistinguishable_candidates_are_marked_ambiguous(self):
        old = [rec("bar", "a", [0], tag="LinearLayout", tags=("FrameLayout",)),
               rec("dou", "a", [0, 0], tag="ImageView")]
        new = [rec("bar", "a", [0], tag="LinearLayout", tags=("FrameLayout",)),
               rec("m1", "a", [0, 1], tag="ImageView"),
               rec("m2", "a", [0, 2], tag="ImageView")]
        rows = ranked(diff_ids.review_rows(["dou"], old, new), "dou")
        self.assertEqual(rows[0][5], "ambiguous")
        self.assertEqual(rows[0][4], rows[1][4])

    def test_id_without_layout_or_any_candidate_gets_no_mapping(self):
        old = composer("uf", ("gpr", "sa3", "bix"))
        rows = list(diff_ids.review_rows(["s0v"], old, composer("uf", ("a", "b", "c"))))
        self.assertEqual(rows, [["s0v", "0", "-", "", "", "none", "", "", "không có trong layout (gán từ code?)"]])
        unrelated = [rec("zz", "elsewhere", [0], tag="WebView", tags=("FrameLayout",), strings={"text": "Help"})]
        self.assertEqual(ranked(diff_ids.review_rows(["sa3"], old, unrelated), "sa3")[0][5], "none")

    def test_confidence_bands(self):
        candidate = diff_ids.Candidate
        self.assertEqual(diff_ids.confidence([candidate("a", 0.9, ""), candidate("b", 0.5, "")]), "high")
        self.assertEqual(diff_ids.confidence([candidate("a", 0.9, ""), candidate("b", 0.8, "")]), "medium")
        self.assertEqual(diff_ids.confidence([candidate("a", 0.5, ""), candidate("b", 0.4, "")]), "low")
        self.assertEqual(diff_ids.confidence([candidate("a", 0.9, ""), candidate("b", 0.88, "")]), "ambiguous")


if __name__ == "__main__":
    unittest.main()
