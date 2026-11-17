"""Text formatting helpers for the status page (tidied: one clear implementation of each)."""

_UNITS = ["B", "KB", "MB", "GB", "TB"]


def format_size(num_bytes):
    value, unit = float(num_bytes), 0
    while value >= 1024 and unit < len(_UNITS) - 1:
        value /= 1024.0
        unit += 1
    return "{} {}".format(int(value) if value == int(value) else round(value, 1), _UNITS[unit])


def format_count(n):
    for limit, suffix in ((10 ** 9, "B"), (10 ** 6, "M"), (10 ** 3, "k")):
        if n >= limit:
            return "{:.1f}{}".format(n / float(limit), suffix)
    return str(n)


def format_thousands(n):
    return "{:,}".format(n)


def plural(n, word):
    return "{} {}{}".format(n, word, "" if n == 1 else "s")


def format_duration(seconds):
    seconds = int(seconds)
    hours, rest = divmod(seconds, 3600)
    minutes, secs = divmod(rest, 60)
    if hours:
        return "{}h {}m".format(hours, minutes) if minutes else "{}h".format(hours)
    if minutes:
        return "{}m {}s".format(minutes, secs) if secs else "{}m".format(minutes)
    return "{}s".format(secs)


def format_uptime(seconds):
    return "up " + format_duration(seconds)


def format_eta(seconds):
    return "about " + format_duration(seconds)


def format_percent(part, whole):
    return "{}%".format(int(100.0 * part / whole)) if whole else "-"


def format_ratio(part, whole):
    return "{} of {}".format(part, whole)


def format_ordinal(n):
    return "{}{}".format(n, {1: "st", 2: "nd", 3: "rd"}.get(n % 10, "th"))


def format_money(cents):
    return "${}.{:02d}".format(cents // 100, cents % 100)


def format_list(items):
    items = list(items)
    return ", ".join(items[:-1]) + (" and " if len(items) > 1 else "") + "".join(items[-1:])


def _convert(factor, places, unit):
    return lambda value: "{:.{}f} {}".format(value * factor, places, unit)


km_to_miles = _convert(0.621371, 1, "mi")
miles_to_km = _convert(1.609344, 1, "km")
kg_to_lb = _convert(2.204623, 1, "lb")
lb_to_kg = _convert(0.453592, 1, "kg")
