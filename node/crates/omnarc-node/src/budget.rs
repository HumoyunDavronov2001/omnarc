//! What this device offers the cluster (REQUIREMENTS Q1, SPEC NO-51): the memory, the cores or the share of the GPU
//! its owner allows, and the hours that applies. The plan is held to it as a hard limit; outside the owner's hours
//! the device offers nothing.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDateTime, Timelike};
use omnarc_protocol::v1;
use prost::Message;

use crate::ring::BoxError;

/// An owner who has not named a budget yet offers half of the device's memory: enough to take part, never so much
/// that the device's own work suffers. The panel and the apps show it and let the owner change it.
pub const DEFAULT_SHARE: f64 = 0.5;

fn path(data: &Path) -> PathBuf {
    data.join("budget.pb")
}

/// The budget the owner set, or none.
#[must_use]
pub fn load(data: &Path) -> Option<v1::Budget> {
    let bytes = std::fs::read(path(data)).ok()?;
    v1::Budget::decode(bytes.as_slice()).ok()
}

/// Keeps the owner's budget.
///
/// # Errors
/// The budget is not one, or the file cannot be written.
pub fn save(data: &Path, budget: &v1::Budget) -> Result<(), BoxError> {
    check(budget)?;
    std::fs::create_dir_all(data)?;
    let file = path(data);
    let partial = file.with_extension("pb.partial");
    std::fs::write(&partial, budget.encode_to_vec())?;
    std::fs::rename(partial, file)?;
    Ok(())
}

/// Refuses a budget that cannot mean anything.
///
/// # Errors
/// A share outside 0..1 or an hour outside the day.
pub fn check(budget: &v1::Budget) -> Result<(), BoxError> {
    if !(0.0..=1.0).contains(&budget.gpu_share) {
        return Err(format!("a GPU share is between 0 and 1, not {}", budget.gpu_share).into());
    }
    for window in &budget.hours {
        if window.start_minute >= 24 * 60 || window.end_minute > 24 * 60 {
            return Err(
                format!("{}..{} is not a time of day in minutes", window.start_minute, window.end_minute).into()
            );
        }
        if window.weekdays >= 1 << 7 {
            return Err("weekdays are seven bits, Monday first".into());
        }
    }
    Ok(())
}

/// Whether `at` falls in one of the owner's windows; no windows: always.
#[must_use]
pub fn in_hours(budget: &v1::Budget, at: NaiveDateTime) -> bool {
    if budget.hours.is_empty() {
        return true;
    }
    let minute = at.hour() * 60 + at.minute();
    let today = at.weekday().num_days_from_monday();
    let yesterday = (today + 6) % 7;
    let on = |weekdays: u32, day: u32| weekdays == 0 || weekdays & (1 << day) != 0;
    budget.hours.iter().any(|w| {
        if w.start_minute <= w.end_minute {
            on(w.weekdays, today) && (w.start_minute..w.end_minute).contains(&minute)
        } else {
            // a window past midnight belongs to the day it starts on
            (on(w.weekdays, today) && minute >= w.start_minute) || (on(w.weekdays, yesterday) && minute < w.end_minute)
        }
    })
}

/// The memory this device offers at `at`: what the owner allows, never more than the device has, nothing outside
/// the owner's hours.
#[must_use]
pub fn offered_memory(budget: Option<&v1::Budget>, total_bytes: u64, at: NaiveDateTime) -> u64 {
    match budget {
        None => {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
            let share = (total_bytes as f64 * DEFAULT_SHARE) as u64;
            share
        }
        Some(budget) if !in_hours(budget, at) => 0,
        Some(budget) if budget.memory_bytes == 0 => total_bytes,
        Some(budget) => budget.memory_bytes.min(total_bytes),
    }
}

/// The cores Omnarc may keep busy now, at least one.
#[must_use]
pub fn offered_cores(budget: Option<&v1::Budget>, cores: u32) -> u32 {
    let cores = cores.max(1);
    match budget {
        Some(budget) if budget.cpu_cores > 0 => budget.cpu_cores.min(cores),
        // half of them by default, like the memory
        None => cores.div_ceil(2),
        Some(_) => cores,
    }
}

/// [`offered_memory`] now, in local time.
#[must_use]
pub fn offered_memory_now(budget: Option<&v1::Budget>, total_bytes: u64) -> u64 {
    offered_memory(budget, total_bytes, Local::now().naive_local())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    const GB: u64 = 1 << 30;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        // October 2026: the 19th is a Monday
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn evenings() -> v1::Budget {
        v1::Budget {
            memory_bytes: 8 * GB,
            hours: vec![v1::HoursWindow { start_minute: 19 * 60, end_minute: 8 * 60, weekdays: 0b1_1111 }],
            ..Default::default()
        }
    }

    #[test]
    fn a_device_offers_what_its_owner_allows_and_no_more_than_it_has() {
        let half_gb = v1::Budget { memory_bytes: GB / 2, ..Default::default() };
        assert_eq!(offered_memory(Some(&half_gb), 2 * GB, at(21, 12, 0)), GB / 2);
        let too_much = v1::Budget { memory_bytes: 64 * GB, ..Default::default() };
        assert_eq!(offered_memory(Some(&too_much), 16 * GB, at(21, 12, 0)), 16 * GB);
        assert_eq!(offered_memory(None, 16 * GB, at(21, 12, 0)), 8 * GB, "half by default");
    }

    #[test]
    fn outside_the_owners_hours_a_device_offers_nothing() {
        let budget = evenings();
        assert_eq!(offered_memory(Some(&budget), 64 * GB, at(21, 12, 0)), 0, "Wednesday noon");
        assert_eq!(offered_memory(Some(&budget), 64 * GB, at(21, 19, 0)), 8 * GB, "Wednesday 19:00");
        assert_eq!(offered_memory(Some(&budget), 64 * GB, at(22, 7, 59)), 8 * GB, "Thursday before 8, from Wednesday");
        assert_eq!(offered_memory(Some(&budget), 64 * GB, at(24, 20, 0)), 0, "Saturday evening is not a weekday");
        assert_eq!(offered_memory(Some(&budget), 64 * GB, at(24, 7, 0)), 8 * GB, "Saturday morning, from Friday");
    }

    #[test]
    fn cores_are_limited_the_same_way() {
        let one = v1::Budget { cpu_cores: 1, ..Default::default() };
        assert_eq!(offered_cores(Some(&one), 8), 1);
        assert_eq!(offered_cores(None, 8), 4);
        assert_eq!(offered_cores(None, 1), 1);
    }

    #[test]
    fn a_budget_survives_a_restart_and_nonsense_is_refused() {
        let dir = std::env::temp_dir().join(format!("omnarc-budget-{}", std::process::id()));
        save(&dir, &evenings()).unwrap();
        assert_eq!(load(&dir), Some(evenings()));
        let nonsense = v1::Budget { gpu_share: 1.5, ..Default::default() };
        assert!(save(&dir, &nonsense).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
