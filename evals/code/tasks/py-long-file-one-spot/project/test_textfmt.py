import unittest

import status
import textfmt


class TextfmtTest(unittest.TestCase):
    def test_size(self):
        self.assertEqual(textfmt.format_size(0), "0 B")
        self.assertEqual(textfmt.format_size(1536), "1.5 KB")

    def test_count(self):
        self.assertEqual(textfmt.format_count(1200), "1.2k")

    def test_duration_under_an_hour(self):
        self.assertEqual(textfmt.format_duration(45), "45s")
        self.assertEqual(textfmt.format_duration(90), "1m 30s")
        self.assertEqual(textfmt.format_duration(300), "5m")

    def test_duration_from_an_hour_up(self):
        self.assertEqual(textfmt.format_duration(3600), "1h")
        self.assertEqual(textfmt.format_duration(5400), "1h 30m")

    def test_status_page(self):
        self.assertEqual(
            status.render(90, 3, 4, 2048, 600),
            "up 1m 30s\njobs 3 of 4 (75%)\ndisk 2 KB\nnext run about 10m")


if __name__ == "__main__":
    unittest.main()
