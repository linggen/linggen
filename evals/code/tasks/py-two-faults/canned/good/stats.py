"""Small statistics helpers for the grade reports."""


def mean(values):
    """The average of a list of numbers."""
    return sum(values) / len(values)


def median(values):
    """The middle value; for an even count, the average of the two middle ones."""
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def spread(values):
    """Highest minus lowest."""
    return max(values) - min(values)
