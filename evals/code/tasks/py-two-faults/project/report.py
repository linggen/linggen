"""One line per class for the grade report."""

import stats


def class_line(name, scores):
    """'<name>: mean <m>, median <m>, spread <s>' - or 'n/a' for a class with no scores yet."""
    if not scores:
        return "{}: n/a".format(name)
    return "{}: mean {:g}, median {}, spread {}".format(
        name, stats.mean(scores), stats.median(scores), stats.spread(scores))


def report(classes):
    """classes: {name: [scores]} -> one line per class, sorted by name."""
    return "\n".join(class_line(name, classes[name]) for name in sorted(classes))
