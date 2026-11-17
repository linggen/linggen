import unittest

import roster


class RosterTest(unittest.TestCase):
    def test_graded_classes_are_listed_sorted(self):
        self.assertEqual(roster.graded({"b": [3], "a": [1, 2]}), ["a", "b"])


if __name__ == "__main__":
    unittest.main()
