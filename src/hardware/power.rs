// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::hardware::pdh::win_pdh::PdhTelemetryState;
use crate::snapshot::types::{EffectivePowerMode, PowerThermalSnapshot};
#[cfg(windows)]
use std::time::Duration;
use std::time::Instant;

#[derive(Default)]
pub struct PowerCollectorState {
    pub last_mode_poll: Option<Instant>,
    pub cached_mode: EffectivePowerMode,
}

pub fn collect_power_thermal_snapshot(
    state: &mut PowerCollectorState,
    pdh_state: Option<&PdhTelemetryState>,
    idle_baseline_w: f64,
    mock: bool,
) -> PowerThermalSnapshot {
    if mock {
        return mock_power_thermal_snapshot(idle_baseline_w);
    }
    #[cfg(windows)]
    {
        collect_windows_power_thermal(state, pdh_state, idle_baseline_w)
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        let _ = pdh_state;
        collect_linux_power_thermal(idle_baseline_w)
    }
}

fn mock_power_thermal_snapshot(idle_baseline_w: f64) -> PowerThermalSnapshot {
    let baseline = if idle_baseline_w > 0.0 {
        idle_baseline_w
    } else {
        4.8
    };
    let package_w = 17.5;
    let power_above_idle_w = (package_w - baseline).max(0.0);

    PowerThermalSnapshot {
        package_w,
        core_pp0_w: 9.4,
        gpu_pp1_w: 4.6,
        dram_w: 2.3,
        power_above_idle_w,
        idle_baseline_w: baseline,
        ac_online: true,
        battery_present: false,
        battery_pct: None,
        discharge_rate_w: None,
        effective_power_mode: EffectivePowerMode::Balanced,
        acpi_thermal_zone_c: Some(46.0),
        gpu_c: Some(44.0),
        nvme_composite_c: Some(41.5),
        nvme_asic_c: Some(45.0),
        nvme_nand_c: Some(39.0),
        cpu_package_dts_c: Some(48.5),
        cpu_core_dts_c: Some(vec![48.0, 49.0, 47.5, 48.2, 44.0, 43.5, 44.2, 43.8]),
        cpu_fan_rpm: Some(2400),
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct SystemBatteryState {
    ac_on_line: u8,
    battery_present: u8,
    charging: u8,
    discharging: u8,
    spare1: [u8; 3],
    tag: u8,
    max_capacity: u32,
    remaining_capacity: u32,
    rate: i32,
    estimated_time: u32,
    default_alert1: u32,
    default_alert2: u32,
}

#[cfg(windows)]
extern "system" {
    fn CallNtPowerInformation(
        information_level: i32,
        input_buffer: *const std::ffi::c_void,
        input_buffer_length: u32,
        output_buffer: *mut std::ffi::c_void,
        output_buffer_length: u32,
    ) -> i32;
}

#[cfg(windows)]
fn collect_windows_power_thermal(
    state: &mut PowerCollectorState,
    pdh_state: Option<&PdhTelemetryState>,
    idle_baseline_w: f64,
) -> PowerThermalSnapshot {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    let has_status = unsafe { GetSystemPowerStatus(&mut status) != 0 };

    let (ac_online, battery_present, battery_pct) = if has_status {
        let ac = status.ACLineStatus == 1;
        let bat_present = (status.BatteryFlag & 128) == 0 && status.BatteryFlag != 255;
        let pct = if status.BatteryLifePercent <= 100 {
            Some(status.BatteryLifePercent)
        } else {
            None
        };
        (ac, bat_present, pct)
    } else {
        (true, false, None)
    };

    let mut discharge_rate_w = None;
    let mut bat_state = SystemBatteryState::default();
    let status_code = unsafe {
        CallNtPowerInformation(
            30, // SystemBatteryState
            std::ptr::null(),
            0,
            &mut bat_state as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<SystemBatteryState>() as u32,
        )
    };
    if status_code == 0 && bat_state.discharging != 0 && bat_state.rate > 0 {
        discharge_rate_w = Some(bat_state.rate as f64 / 1000.0);
    }

    let mut package_w = 0.0;
    let mut core_pp0_w = 0.0;
    let mut gpu_pp1_w = 0.0;
    let mut dram_w = 0.0;
    let mut acpi_thermal_zone_c = None;

    if let Some(pdh) = pdh_state {
        if let Some(mw) = pdh.read_double("rapl_pkg") {
            package_w = mw / 1000.0;
        }
        if let Some(mw) = pdh.read_double("rapl_pp0") {
            core_pp0_w = mw / 1000.0;
        }
        if let Some(mw) = pdh.read_double("rapl_pp1") {
            gpu_pp1_w = mw / 1000.0;
        }
        if let Some(mw) = pdh.read_double("rapl_dram") {
            dram_w = mw / 1000.0;
        }

        let thermal_items = pdh.read_array("thermals");
        if let Some((_, raw_k_tenths)) = thermal_items.first() {
            let c = (*raw_k_tenths / 10.0) - 273.15;
            if (0.0..=120.0).contains(&c) {
                acpi_thermal_zone_c = Some(c as f32);
            }
        }
    }

    let should_poll_mode = state
        .last_mode_poll
        .map(|t| t.elapsed() >= Duration::from_secs(2))
        .unwrap_or(true);
    if should_poll_mode {
        state.cached_mode = query_effective_power_mode();
        state.last_mode_poll = Some(Instant::now());
    }
    let effective_power_mode = state.cached_mode;

    let baseline = if idle_baseline_w > 0.0 {
        idle_baseline_w
    } else {
        4.8
    };
    let power_above_idle_w = (package_w - baseline).max(0.0);

    PowerThermalSnapshot {
        package_w,
        core_pp0_w,
        gpu_pp1_w,
        dram_w,
        power_above_idle_w,
        idle_baseline_w: baseline,
        ac_online,
        battery_present,
        battery_pct,
        discharge_rate_w,
        effective_power_mode,
        acpi_thermal_zone_c,
        gpu_c: None,
        nvme_composite_c: None,
        nvme_asic_c: None,
        nvme_nand_c: None,
        cpu_package_dts_c: None,
        cpu_core_dts_c: None,
        cpu_fan_rpm: None,
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct WinGuid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[cfg(windows)]
fn query_effective_power_mode() -> EffectivePowerMode {
    type PowerGetEffectiveOverlaySchemeFn = unsafe extern "system" fn(guid: *mut WinGuid) -> u32;

    static FUNC: std::sync::OnceLock<Option<PowerGetEffectiveOverlaySchemeFn>> =
        std::sync::OnceLock::new();

    let func = FUNC.get_or_init(|| unsafe {
        let lib = libloading::Library::new("powrprof.dll").ok()?;
        let symbol: libloading::Symbol<PowerGetEffectiveOverlaySchemeFn> =
            lib.get(b"PowerGetEffectiveOverlayScheme\0").ok()?;
        let fn_ptr = *symbol;
        std::mem::forget(lib);
        Some(fn_ptr)
    });

    if let Some(get_overlay) = func {
        let mut guid = WinGuid::default();
        if unsafe { get_overlay(&mut guid) } == 0 {
            if guid.data1 == 0xDED574B5 {
                return EffectivePowerMode::BestPerformance;
            }
            if guid.data1 == 0x961CC777 {
                return EffectivePowerMode::BestPowerEfficiency;
            }
        }
    }
    EffectivePowerMode::Balanced
}

#[cfg(not(windows))]
fn collect_linux_power_thermal(idle_baseline_w: f64) -> PowerThermalSnapshot {
    mock_power_thermal_snapshot(idle_baseline_w)
}
