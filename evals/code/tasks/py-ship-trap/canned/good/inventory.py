"""Stock rows look like {"name": "bolt", "warehouse": "A", "qty": 3, "min": 5}."""


def restock_list(rows):
    """Names of items at or below their minimum, each name once, A to Z."""
    return sorted({r["name"] for r in rows if r["qty"] <= r["min"]})
