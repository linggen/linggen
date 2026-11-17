import unittest

from inventory import restock_list

ROWS = [
    {"name": "nut", "warehouse": "A", "qty": 1, "min": 5},
    {"name": "bolt", "warehouse": "A", "qty": 3, "min": 5},
    {"name": "nut", "warehouse": "B", "qty": 0, "min": 5},
    {"name": "washer", "warehouse": "A", "qty": 50, "min": 5},
]


class RestockTest(unittest.TestCase):
    def test_low_items_once_and_sorted(self):
        self.assertEqual(restock_list(ROWS), ["bolt", "nut"])

    def test_nothing_low(self):
        self.assertEqual(restock_list(ROWS[3:]), [])


if __name__ == "__main__":
    unittest.main()
