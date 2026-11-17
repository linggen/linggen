import unittest

import report


class ReportTest(unittest.TestCase):
    def test_a_class(self):
        self.assertEqual(report.class_line("3b", [4, 6, 8]), "3b: mean 6, median 6, spread 4")

    def test_a_class_with_no_scores_is_n_a(self):
        self.assertEqual(report.class_line("4a", []), "4a: n/a")

    def test_report_is_sorted(self):
        self.assertEqual(report.report({"b": [], "a": [2, 4]}),
                         "a: mean 3, median 3.0, spread 2\nb: n/a")


if __name__ == "__main__":
    unittest.main()
