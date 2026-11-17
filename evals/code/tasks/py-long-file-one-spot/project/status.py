"""The status page: a few lines of text built from textfmt."""

import textfmt


def render(uptime_s, jobs_done, jobs_total, disk_bytes, next_run_in_s):
    lines = [
        textfmt.format_uptime(uptime_s),
        "jobs " + textfmt.format_ratio(jobs_done, jobs_total) + " (" + textfmt.format_percent(jobs_done, jobs_total) + ")",
        "disk " + textfmt.format_size(disk_bytes),
        "next run " + textfmt.format_eta(next_run_in_s),
    ]
    return "\n".join(lines)
