"""python3 -m shop.cli invoice.json  ->  the invoice as text."""

import json
import sys

from . import render


def main(argv):
    with open(argv[1]) as f:
        lines = json.load(f)
    print(render.invoice_text(lines))


if __name__ == "__main__":
    main(sys.argv)
