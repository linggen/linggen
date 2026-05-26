//! Cron expression validation + parsing for mission schedules.
//!
//! Linggen accepts 5-field cron (`min hour dom month dow`); the
//! underlying `cron` crate wants 7. This module owns that
//! translation plus the dow normalization (Sunday-as-0 → 7) and
//! exposes two functions the rest of the codebase calls:
//! `validate_cron` (yes/no) and `parse_cron` (typed schedule).

use anyhow::{bail, Result};

/// Convert a 5-field cron expression to the 7-field format the
/// `cron` crate expects. Normalizes Sunday (`0`) to ISO weekday
/// `7`, splitting ranges like `0-5` into `1-5,7` so the underlying
/// parser doesn't reject them.
pub(super) fn to_seven_field(schedule: &str) -> Result<String> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        bail!(
            "Invalid cron expression '{}': expected 5 fields (min hour dom month dow)",
            schedule
        );
    }
    let dow = fields[4]
        .split(',')
        .flat_map(|part| {
            if let Some((start_s, end_s)) = part.split_once('-') {
                let start_num = start_s.trim().parse::<u8>().ok();
                let end_num = end_s.trim().parse::<u8>().ok();
                match (start_num, end_num) {
                    (Some(0), Some(e)) if e >= 1 => {
                        vec![format!("1-{}", e), "7".to_string()]
                    }
                    (Some(s), Some(0)) if s >= 1 => {
                        vec![format!("{}-7", s)]
                    }
                    _ => vec![part.to_string()],
                }
            } else if part.trim() == "0" {
                vec!["7".to_string()]
            } else {
                vec![part.to_string()]
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    Ok(format!(
        "0 {} {} {} {} {} *",
        fields[0], fields[1], fields[2], fields[3], dow
    ))
}

pub fn validate_cron(schedule: &str) -> Result<()> {
    let seven = to_seven_field(schedule)?;
    seven.parse::<cron::Schedule>().map_err(|e| {
        anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e)
    })?;
    Ok(())
}

pub fn parse_cron(schedule: &str) -> Result<cron::Schedule> {
    let seven = to_seven_field(schedule)?;
    seven
        .parse::<cron::Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e))
}
