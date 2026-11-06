use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Priority {
    High,
    #[default]
    Normal,
    Low,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MissedPolicy {
    #[default]
    RunWhenPossible,
    Ask,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Schedule {
    #[default]
    Immediate,
    Once {
        at: u64,
        timezone: String,
        missed: MissedPolicy,
        grace_minutes: u16,
    },
    Window {
        start_minute: u16,
        end_minute: u16,
        weekdays: Vec<u8>,
        timezone: String,
    },
}

impl Schedule {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Immediate => Ok(()),
            Self::Once {
                at,
                timezone,
                grace_minutes,
                ..
            } => {
                if *at == 0 || *at > 253_402_300_799_000 || *grace_minutes > 10_080 {
                    return Err("Choose a valid date and a grace period up to one week.".into());
                }
                wall_clock(timezone, *at).map(|_| ())
            }
            Self::Window {
                start_minute,
                end_minute,
                weekdays,
                timezone,
            } => {
                if *start_minute >= 1440
                    || *end_minute >= 1440
                    || start_minute == end_minute
                    || weekdays.is_empty()
                    || weekdays.iter().any(|day| *day > 6)
                {
                    return Err(
                        "Choose different start and end times and at least one weekday.".into(),
                    );
                }
                wall_clock(timezone, crate::queue::now()).map(|_| ())
            }
        }
    }

    pub fn needs_review(&self, stamp: u64) -> bool {
        matches!(self, Self::Once { at, missed: MissedPolicy::Ask, grace_minutes, .. }
            if stamp > at.saturating_add(u64::from(*grace_minutes) * 60_000))
    }

    pub fn eligible(&self, stamp: u64) -> bool {
        match self {
            Self::Immediate => true,
            Self::Once { at, .. } => stamp >= *at && !self.needs_review(stamp),
            Self::Window {
                start_minute,
                end_minute,
                weekdays,
                timezone,
            } => wall_clock(timezone, stamp)
                .is_ok_and(|clock| in_window(*start_minute, *end_minute, weekdays, clock)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct WallClock {
    minute: u16,
    weekday: u8,
}

fn in_window(start: u16, end: u16, weekdays: &[u8], clock: WallClock) -> bool {
    if start < end {
        weekdays.contains(&clock.weekday) && (start..end).contains(&clock.minute)
    } else if clock.minute >= start {
        weekdays.contains(&clock.weekday)
    } else {
        clock.minute < end && weekdays.contains(&((clock.weekday + 6) % 7))
    }
}

pub fn compare(left: &crate::queue::Job, right: &crate::queue::Job, stamp: u64) -> Ordering {
    let rank = |job: &crate::queue::Job| {
        let base = match job.priority {
            Priority::High => 4,
            Priority::Normal => 2,
            Priority::Low => 0,
        };
        base + stamp.saturating_sub(job.created_at) / (15 * 60_000)
    };
    rank(right)
        .cmp(&rank(left))
        .then_with(|| left.manual_order.cmp(&right.manual_order))
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn wall_clock(zone: &str, stamp: u64) -> Result<WallClock, String> {
    if zone == "UTC" {
        return Ok(WallClock {
            minute: ((stamp / 60_000) % 1440) as u16,
            weekday: (((stamp / 86_400_000) + 3) % 7) as u8,
        });
    }
    #[cfg(windows)]
    return windows_time::clock(zone, stamp);
    #[cfg(not(windows))]
    Err("This build supports recurring windows in UTC only.".into())
}

#[tauri::command]
pub fn local_timezone() -> Result<String, String> {
    #[cfg(windows)]
    return windows_time::local_zone();
    #[cfg(not(windows))]
    Ok("UTC".into())
}

#[cfg(windows)]
mod windows_time {
    use super::WallClock;
    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
    };

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Zone {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
        key: [u16; 128],
        dynamic_disabled: u8,
    }
    impl Default for Zone {
        fn default() -> Self {
            Self {
                bias: 0,
                standard_name: [0; 32],
                standard_date: SystemTime::default(),
                standard_bias: 0,
                daylight_name: [0; 32],
                daylight_date: SystemTime::default(),
                daylight_bias: 0,
                key: [0; 128],
                dynamic_disabled: 0,
            }
        }
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDynamicTimeZoneInformation(zone: *mut Zone) -> u32;
        fn EnumDynamicTimeZoneInformation(index: u32, zone: *mut Zone) -> u32;
        fn FileTimeToSystemTime(file: *const FileTime, system: *mut SystemTime) -> i32;
        fn SystemTimeToTzSpecificLocalTimeEx(
            zone: *const Zone,
            utc: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }
    fn key(zone: &Zone) -> String {
        let end = zone
            .key
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(zone.key.len());
        String::from_utf16_lossy(&zone.key[..end])
    }
    pub(super) fn local_zone() -> Result<String, String> {
        let mut zone = Zone::default();
        if unsafe { GetDynamicTimeZoneInformation(&mut zone) } == u32::MAX {
            return Err("Could not read the Windows time zone.".into());
        }
        let name = key(&zone);
        if name.is_empty() {
            return Err("The Windows time zone has no stable identifier. Choose UTC.".into());
        }
        Ok(name)
    }
    fn find(name: &str) -> Result<Zone, String> {
        static CACHE: OnceLock<Mutex<HashMap<String, Zone>>> = OnceLock::new();
        let mut cache = CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        if let Some(zone) = cache.get(name) {
            return Ok(*zone);
        }
        if name.len() > 128 {
            return Err("Invalid time zone.".into());
        }
        for index in 0..1024 {
            let mut zone = Zone::default();
            if unsafe { EnumDynamicTimeZoneInformation(index, &mut zone) } != 0 {
                break;
            }
            if key(&zone) == name {
                cache.insert(name.into(), zone);
                return Ok(zone);
            }
        }
        Err("Choose a Windows time zone identifier or UTC.".into())
    }
    pub(super) fn clock(name: &str, stamp: u64) -> Result<WallClock, String> {
        let zone = find(name)?;
        let ticks = stamp
            .checked_mul(10_000)
            .and_then(|t| t.checked_add(116_444_736_000_000_000))
            .ok_or("Invalid date.")?;
        let file = FileTime {
            low: ticks as u32,
            high: (ticks >> 32) as u32,
        };
        let mut utc = SystemTime::default();
        let mut local = SystemTime::default();
        if unsafe { FileTimeToSystemTime(&file, &mut utc) } == 0
            || unsafe { SystemTimeToTzSpecificLocalTimeEx(&zone, &utc, &mut local) } == 0
        {
            return Err("Could not apply the saved time zone.".into());
        }
        Ok(WallClock {
            minute: local.hour * 60 + local.minute,
            weekday: ((local.weekday + 6) % 7) as u8,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_off_runs_once_by_job_state_and_missed_policy_is_explicit() {
        let mut once = Schedule::Once {
            at: 1_000_000,
            timezone: "UTC".into(),
            missed: MissedPolicy::Ask,
            grace_minutes: 10,
        };
        assert!(!once.eligible(999_999));
        assert!(once.eligible(1_000_000));
        assert!(once.eligible(1_600_000));
        assert!(!once.eligible(1_600_001));
        assert!(once.needs_review(1_600_001));
        if let Schedule::Once { missed, .. } = &mut once {
            *missed = MissedPolicy::RunWhenPossible;
        }
        assert!(once.eligible(9_000_000));
    }

    #[test]
    fn overnight_window_uses_the_day_that_opened_it_and_excludes_the_end() {
        assert!(in_window(
            22 * 60,
            6 * 60,
            &[0],
            WallClock {
                minute: 23 * 60,
                weekday: 0
            }
        ));
        assert!(in_window(
            22 * 60,
            6 * 60,
            &[0],
            WallClock {
                minute: 5 * 60,
                weekday: 1
            }
        ));
        assert!(!in_window(
            22 * 60,
            6 * 60,
            &[0],
            WallClock {
                minute: 6 * 60,
                weekday: 1
            }
        ));
        assert!(!in_window(
            22 * 60,
            6 * 60,
            &[0],
            WallClock {
                minute: 5 * 60,
                weekday: 0
            }
        ));
        assert!(in_window(
            60,
            120,
            &[4],
            WallClock {
                minute: 60,
                weekday: 4
            }
        ));
        assert!(!in_window(
            60,
            120,
            &[4],
            WallClock {
                minute: 120,
                weekday: 4
            }
        ));
    }

    #[test]
    fn utc_window_and_invalid_rules() {
        let window = Schedule::Window {
            start_minute: 60,
            end_minute: 120,
            weekdays: vec![3],
            timezone: "UTC".into(),
        };
        assert!(window.eligible(3_600_000));
        assert!(!window.eligible(7_200_000));
        assert!(window.validate().is_ok());
        let invalid = Schedule::Window {
            start_minute: 0,
            end_minute: 0,
            weekdays: vec![7],
            timezone: "UTC".into(),
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn old_low_priority_jobs_eventually_pass_new_high_priority_jobs() {
        let make_job = |id: &str, priority: &str, created: u64, order: i64| {
            serde_json::from_value::<crate::queue::Job>(serde_json::json!({
                "id":id,"runId":0,"clientRequest":"schedule-fixture","title":"Fixture",
                "createdAt":created,"updatedAt":created,"state":"pending",
                "source":{"kind":"transform","libraryId":"local","options":{"kind":"trim","start":0,"end":1,"precise":false}},
                "folder":"C:/Downloads","provider":"FFmpeg","thumbnail":null,"progress":null,
                "outputs":[],"error":null,"attempt":0,"groupId":null,"priority":priority,"manualOrder":order
            })).unwrap()
        };
        let old_low = make_job("old", "low", 0, 0);
        let new_high = make_job("new", "high", 5_400_000, 0);
        assert_eq!(compare(&old_low, &new_high, 5_400_000), Ordering::Less);
        let earlier_manual = make_job("manual", "normal", 5_400_000, -1);
        let later_manual = make_job("later", "normal", 5_400_000, 0);
        assert_eq!(
            compare(&earlier_manual, &later_manual, 5_400_000),
            Ordering::Less
        );
    }

    #[cfg(windows)]
    #[test]
    fn saved_windows_zone_applies_spring_gap_and_autumn_repeat() {
        // 2026-03-29 00:30/01:30 UTC and 2026-10-25 00:30/01:30 UTC.
        let spring_before = wall_clock("Romance Standard Time", 1_774_744_200_000).unwrap();
        let spring_after = wall_clock("Romance Standard Time", 1_774_747_800_000).unwrap();
        assert_eq!(spring_before.minute, 90);
        assert_eq!(spring_after.minute, 210);
        let autumn_before = wall_clock("Romance Standard Time", 1_792_888_200_000).unwrap();
        let autumn_after = wall_clock("Romance Standard Time", 1_792_891_800_000).unwrap();
        assert_eq!(autumn_before.minute, 150);
        assert_eq!(autumn_after.minute, 150);
    }
}
