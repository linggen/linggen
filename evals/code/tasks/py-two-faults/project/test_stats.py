import unittest

import stats


class StatsTest(unittest.TestCase):
    def test_mean_of_whole_result(self):
        self.assertEqual(stats.mean([2, 4, 6]), 4)

    def test_mean_with_a_fraction(self):
        self.assertEqual(stats.mean([1, 2]), 1.5)

    def test_median(self):
        self.assertEqual(stats.median([5, 1, 3]), 3)
        self.assertEqual(stats.median([1, 2, 3, 4]), 2.5)

    def test_spread(self):
        self.assertEqual(stats.spread([4, 9, 1]), 8)


if __name__ == "__main__":
    unittest.main()
