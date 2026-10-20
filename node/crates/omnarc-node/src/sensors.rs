//! What a device says about its heat and its power in its telemetry: the panel's outer markers (PLAN.md §9.4), and from
//! stage B5 the limits that hand work to a replica when a device runs hot or low (Q5).

use std::path::Path;

use omnarc_protocol::v1;

/// The device's temperature sensors.
#[derive(Debug)]
pub struct Sensors {
    components: sysinfo::Components,
}

impl Default for Sensors {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensors {
    #[must_use]
    pub fn new() -> Self {
        Self { components: sysinfo::Components::new_with_refreshed_list() }
    }

    /// How hot the device runs, from its hottest sensor; unknown without sensors.
    pub fn thermal(&mut self) -> v1::ThermalState {
        self.components.refresh(false);
        let hottest = self
            .components
            .iter()
            .filter_map(|c| c.temperature().map(|t| (t, c.critical())))
            .filter(|(t, _)| t.is_finite() && *t > 0.0)
            .max_by(|a, b| a.0.total_cmp(&b.0));
        match hottest {
            Some((celsius, critical)) => thermal_state(celsius, critical),
            None => v1::ThermalState::Unspecified,
        }
    }
}

/// The thermal state of a sensor at `celsius`: against its critical point when it has one, else against common limits.
#[must_use]
pub fn thermal_state(celsius: f32, critical: Option<f32>) -> v1::ThermalState {
    let critical = critical.filter(|c| c.is_finite() && *c > 40.0).unwrap_or(100.0);
    if celsius >= critical - 5.0 {
        v1::ThermalState::Critical
    } else if celsius >= critical - 15.0 {
        v1::ThermalState::Serious
    } else if celsius >= critical - 30.0 {
        v1::ThermalState::Fair
    } else {
        v1::ThermalState::Nominal
    }
}

/// One power supply as the system describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Supply {
    /// "Battery", "Mains", "USB", ...
    pub kind: String,
    /// "Charging", "Discharging", "Full", "Not charging", ...
    pub status: String,
    /// 0–100, for a battery.
    pub capacity: Option<u8>,
    /// For a mains adapter: whether it is plugged in.
    pub online: Option<bool>,
}

/// Where the device's power comes from: on a battery, charging, or plugged in without one.
#[must_use]
pub fn power_state(supplies: &[Supply]) -> v1::PowerState {
    let batteries: Vec<&Supply> = supplies.iter().filter(|s| s.kind == "Battery" && s.capacity.is_some()).collect();
    let plugged = supplies.iter().any(|s| s.kind != "Battery" && s.online == Some(true));
    if batteries.is_empty() {
        return v1::PowerState { on_battery: false, charging: false, battery_fraction: 1.0 };
    }
    // a device has one battery or two: their mean
    let (sum, count) = batteries
        .iter()
        .fold((0.0f32, 0.0f32), |(sum, count), b| (sum + f32::from(b.capacity.unwrap_or(0)), count + 1.0));
    let fraction = sum / (100.0 * count);
    let charging = batteries.iter().any(|b| b.status == "Charging");
    v1::PowerState { on_battery: !plugged && !charging, charging, battery_fraction: fraction.clamp(0.0, 1.0) }
}

/// The power supplies of this device: Linux and Android say it in `/sys/class/power_supply`; Windows answers
/// [`power`] itself, and on the other systems the apps of stage B5 will.
#[must_use]
pub fn supplies() -> Vec<Supply> {
    let root = Path::new("/sys/class/power_supply");
    let Ok(entries) = std::fs::read_dir(root) else { return vec![] };
    let read = |dir: &Path, name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_owned());
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let dir = entry.path();
            Supply {
                kind: read(&dir, "type").unwrap_or_default(),
                status: read(&dir, "status").unwrap_or_default(),
                capacity: read(&dir, "capacity").and_then(|c| c.parse().ok()),
                online: read(&dir, "online").and_then(|o| o.parse::<u8>().ok()).map(|o| o == 1),
            }
        })
        .collect()
}

/// This device's power, when it says.
#[must_use]
pub fn power() -> Option<v1::PowerState> {
    #[cfg(windows)]
    {
        windows::power()
    }
    #[cfg(not(windows))]
    {
        let supplies = supplies();
        (!supplies.is_empty()).then(|| power_state(&supplies))
    }
}

/// The power state in Windows' terms (`SYSTEM_POWER_STATUS`): `ac_line` 0 off, 1 on, 255 unknown; `battery_flag` a
/// set of 1 high, 2 low, 4 critical and 8 charging, 128 without a battery and 255 unknown; `percent` 0–100, 255
/// unknown. A device without a battery runs on the mains, as on Linux.
#[must_use]
pub fn windows_power_state(ac_line: u8, battery_flag: u8, percent: u8) -> Option<v1::PowerState> {
    match battery_flag {
        255 => None,
        128 => Some(v1::PowerState { on_battery: false, charging: false, battery_fraction: 1.0 }),
        flag => {
            let charging = flag & 8 != 0;
            let fraction = if percent <= 100 { f32::from(percent) / 100.0 } else { 0.0 };
            Some(v1::PowerState { on_battery: ac_line == 0 && !charging, charging, battery_fraction: fraction })
        }
    }
}

#[cfg(windows)]
mod windows {
    #![allow(unsafe_code)]

    use omnarc_protocol::v1;
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    pub fn power() -> Option<v1::PowerState> {
        // SAFETY: SYSTEM_POWER_STATUS is plain data, all zeros a valid value
        let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        // SAFETY: the pointer is to a live, writable SYSTEM_POWER_STATUS that the call only fills in
        if unsafe { GetSystemPowerStatus(&raw mut status) } == 0 {
            return None;
        }
        super::windows_power_state(status.ACLineStatus, status.BatteryFlag, status.BatteryLifePercent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_says_where_the_power_comes_from() {
        // plugged in and charging, plugged in and full, on battery and low, a desktop, and nothing known
        let charging = windows_power_state(1, 8 | 1, 81).unwrap();
        assert!(charging.charging && !charging.on_battery && (charging.battery_fraction - 0.81).abs() < 1e-6);
        let full = windows_power_state(1, 1, 100).unwrap();
        assert!(!full.charging && !full.on_battery);
        let low = windows_power_state(0, 2, 18).unwrap();
        assert!(low.on_battery && !low.charging && (low.battery_fraction - 0.18).abs() < 1e-6);
        assert_eq!(
            windows_power_state(1, 128, 255),
            Some(v1::PowerState { on_battery: false, charging: false, battery_fraction: 1.0 })
        );
        assert_eq!(windows_power_state(255, 255, 255), None);
        assert_eq!(windows_power_state(0, 1, 255).unwrap().battery_fraction, 0.0);
    }

    fn supply(kind: &str, status: &str, capacity: Option<u8>, online: Option<bool>) -> Supply {
        Supply { kind: kind.into(), status: status.into(), capacity, online }
    }

    #[test]
    fn heat_is_measured_against_the_critical_point() {
        assert_eq!(thermal_state(45.0, Some(100.0)), v1::ThermalState::Nominal);
        assert_eq!(thermal_state(72.0, Some(100.0)), v1::ThermalState::Fair);
        assert_eq!(thermal_state(88.0, Some(100.0)), v1::ThermalState::Serious);
        assert_eq!(thermal_state(97.0, Some(100.0)), v1::ThermalState::Critical);
        // a phone's battery sensor is critical far lower
        assert_eq!(thermal_state(52.0, Some(60.0)), v1::ThermalState::Serious);
        // without a critical point, common limits
        assert_eq!(thermal_state(90.0, None), v1::ThermalState::Serious);
    }

    #[test]
    fn power_says_where_it_comes_from() {
        let laptop = [supply("Mains", "", None, Some(true)), supply("Battery", "Charging", Some(93), None)];
        let state = power_state(&laptop);
        assert!(state.charging && !state.on_battery);
        assert!((state.battery_fraction - 0.93).abs() < 1e-6);

        let unplugged = [supply("Mains", "", None, Some(false)), supply("Battery", "Discharging", Some(41), None)];
        assert!(power_state(&unplugged).on_battery);

        let desktop = [supply("USB", "Not charging", None, Some(false))];
        let state = power_state(&desktop);
        assert!(!state.on_battery && !state.charging && (state.battery_fraction - 1.0).abs() < f32::EPSILON);
    }
}
