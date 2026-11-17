import unittest

import report
import roster
import stats


class HiddenTest(unittest.TestCase):
    def test_mean_is_exact(self):
        self.assertEqual(stats.mean([1, 2]), 1.5)
        self.assertEqual(stats.mean([1, 1, 2]), 4 / 3)
        self.assertEqual(stats.mean([7]), 7)
        self.assertEqual(stats.mean([-3, 0]), -1.5)

    def test_report_lines(self):
        self.assertEqual(report.class_line("5c", [1, 2]), "5c: mean 1.5, median 1.5, spread 1")
        self.assertEqual(report.class_line("5d", [3, 4, 4]), "5d: mean 3.66667, median 4, spread 1")
        self.assertEqual(report.class_line("4a", []), "4a: n/a")

    def test_an_empty_class_is_not_graded(self):
        # roster relies on stats.mean([]) failing: no scores, no mean, not graded.
        self.assertEqual(roster.graded({"z": [], "a": [10, 20], "m": []}), ["a"])
        self.assertEqual(roster.graded({}), [])


if __name__ == "__main__":
    unittest.main()
