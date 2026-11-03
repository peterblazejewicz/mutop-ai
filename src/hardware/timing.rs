// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::snapshot::types::{SystemTimingSnapshot, TimerResolutionSnapshot};

pub fn collect_timing_snapshot(mock: bool) -> SystemTimingSnapshot {
    if mock {
        return mock_timing_snapshot();
    }

    #[cfg(windows)]
    {
        collect_windows_timing()
    }

    #[cfg(not(windows))]
    {
        collect_linux_timing()
    }
}

fn mock_timing_snapshot() -> SystemTimingSnapshot {
    SystemTimingSnapshot {
        tsc_invariant: true,
        crystal_hz: 38_400_000,
        tsc_hz: 3_300_000_000,
        qpc_frequency_hz: 10_000_000,
        unbiased_uptime_sec: 14250.5,
        timer_resolution: TimerResolutionSnapshot {
            current_ms: 1.0,
            minimum_ms: 0.5,
            maximum_ms: 15.625,
            is_high_resolution_1ms: true,
        },
        adjustment_disabled: false,
        frequency_slew_ratio: 1.0,
        rtc_battery_low: Some(false),
    }
}

#[cfg(windows)]
extern "system" {
    fn NtQueryTimerResolution(
        maximum_time: *mut u32,
        minimum_time: *mut u32,
        current_time: *mut u32,
    ) -> i32;

    fn QueryPerformanceFrequency(lp_frequency: *mut i64) -> i32;
    fn QueryPerformanceCounter(lp_performance_count: *mut i64) -> i32;
}

#[cfg(windows)]
fn collect_windows_timing() -> SystemTimingSnapshot {
    let mut max_time: u32 = 156250;
    let mut min_time: u32 = 5000;
    let mut cur_time: u32 = 156250;

    unsafe {
        let _ = NtQueryTimerResolution(&mut max_time, &mut min_time, &mut cur_time);
    }

    let current_ms = cur_time as f64 / 10000.0;
    let minimum_ms = min_time as f64 / 10000.0;
    let maximum_ms = max_time as f64 / 10000.0;
    let is_high_resolution_1ms = current_ms <= 1.05;

    let mut qpc_freq: i64 = 10_000_000;
    let mut qpc_count: i64 = 0;
    unsafe {
        QueryPerformanceFrequency(&mut qpc_freq);
        QueryPerformanceCounter(&mut qpc_count);
    }
    let unbiased_uptime_sec = if qpc_freq > 0 {
        qpc_count as f64 / qpc_freq as f64
    } else {
        0.0
    };

    let (tsc_invariant, crystal_hz, tsc_hz) = probe_cpuid_timing();

    SystemTimingSnapshot {
        tsc_invariant,
        crystal_hz,
        tsc_hz,
        qpc_frequency_hz: qpc_freq as u64,
        unbiased_uptime_sec,
        timer_resolution: TimerResolutionSnapshot {
            current_ms,
            minimum_ms,
            maximum_ms,
            is_high_resolution_1ms,
        },
        adjustment_disabled: false,
        frequency_slew_ratio: 1.0,
        rtc_battery_low: None,
    }
}

#[cfg(windows)]
fn probe_cpuid_timing() -> (bool, u64, u64) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::__cpuid;
        // Check Invariant TSC: Leaf 0x80000007, EDX bit 8
        let ext_info = __cpuid(0x80000007);
        let tsc_invariant = (ext_info.edx & (1 << 8)) != 0;

        // Check Leaf 0x15 for Core Crystal Clock (Lunar Lake is 38.4 MHz = 38,400,000 Hz)
        let tsc_info = __cpuid(0x15);
        let crystal_hz = if tsc_info.ecx > 0 {
            tsc_info.ecx as u64
        } else {
            38_400_000
        };

        let tsc_hz = if tsc_info.eax > 0 && tsc_info.ebx > 0 {
            (crystal_hz * tsc_info.ebx as u64) / tsc_info.eax as u64
        } else {
            3_300_000_000
        };

        (tsc_invariant, crystal_hz, tsc_hz)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        (true, 38_400_000, 3_300_000_000)
    }
}

#[cfg(not(windows))]
fn collect_linux_timing() -> SystemTimingSnapshot {
    mock_timing_snapshot()
}
