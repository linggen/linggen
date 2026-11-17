"""Which classes have grades yet."""

import stats


def graded(classes):
    """Names of the classes that have scores, sorted. A class counts as graded
    when it has a mean."""
    names = []
    for name in sorted(classes):
        try:
            stats.mean(classes[name])
        except ZeroDivisionError:
            continue
        names.append(name)
    return names
