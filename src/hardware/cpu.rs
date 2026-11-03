// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::snapshot::types::{CoreMicroarch, LunarLakeCpuSnapshot};

#[derive(Default)]
pub struct CpuCollectorState {
    pub prev_ticks: Vec<CoreTickRecord>,
    pub topology: Vec<CoreMicroarch>,
}

#[derive(Clone, Copy, Default)]
pub struct CoreTickRecord {
    pub user: u64,
    pub kernel: u64,
    pub idle: u64,
    pub dpc: u64,
    pub interrupt: u64,
}

pub fn collect_cpu_snapshot(state: &mut CpuCollectorState, mock: bool) -> LunarLakeCpuSnapshot {
    if mock {
        return mock_cpu_snapshot();
    }

    #[cfg(windows)]
    {
        collect_windows_cpu(state)
    }

    #[cfg(not(windows))]
    {
        collect_linux_cpu(state)
    }
}

fn mock_cpu_snapshot() -> LunarLakeCpuSnapshot {
    let per_core_microarch = vec![
        CoreMicroarch::LionCoveP,
        CoreMicroarch::LionCoveP,
        CoreMicroarch::LionCoveP,
        CoreMicroarch::LionCoveP,
        CoreMicroarch::SkymontLpE,
        CoreMicroarch::SkymontLpE,
        CoreMicroarch::SkymontLpE,
        CoreMicroarch::SkymontLpE,
    ];
    let per_core_pct = vec![38.5, 41.2, 32.0, 34.1, 15.2, 18.0, 14.5, 16.3];
    let per_core_effective_mhz = vec![4600, 4800, 4400, 4500, 3200, 3400, 3100, 3300];
    let per_core_current_mhz = vec![4400, 4400, 4400, 4400, 3100, 3100, 3100, 3100];
    let per_core_max_mhz = vec![4800, 4800, 4800, 4800, 3700, 3700, 3700, 3700];
    let per_core_limit_mhz = vec![4800, 4800, 4800, 4800, 3700, 3700, 3700, 3700];

    let p_load: f32 = per_core_pct[0..4].iter().sum::<f32>() / 4.0;
    let lpe_load: f32 = per_core_pct[4..8].iter().sum::<f32>() / 4.0;
    let total_load: f32 = per_core_pct.iter().sum::<f32>() / 8.0;

    LunarLakeCpuSnapshot {
        total_load_pct: total_load,
        lion_cove_p_load_pct: p_load,
        skymont_lpe_load_pct: lpe_load,
        per_core_pct,
        per_core_effective_mhz,
        per_core_current_mhz,
        per_core_max_mhz,
        per_core_limit_mhz,
        per_core_microarch,
        is_throttled: false,
        user_pct: 20.4,
        system_pct: 5.8,
        idle_pct: 73.8,
        iowait_pct: 0.0,
        dpc_pct: Some(0.8),
        interrupt_pct: Some(0.4),
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SystemProcessorPerformanceInformation {
    idle_time: i64,
    kernel_time: i64,
    user_time: i64,
    dpc_time: i64,
    interrupt_time: i64,
    interrupt_count: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessorPowerInformation {
    number: u32,
    max_mhz: u32,
    current_mhz: u32,
    mhz_limit: u32,
    max_idle_state: u32,
    current_idle_state: u32,
}

#[cfg(windows)]
extern "system" {
    fn NtQuerySystemInformation(
        system_information_class: u32,
        system_information: *mut std::ffi::c_void,
        system_information_length: u32,
        return_length: *mut u32,
    ) -> i32;

    fn CallNtPowerInformation(
        information_level: i32,
        input_buffer: *const std::ffi::c_void,
        input_buffer_length: u32,
        output_buffer: *mut std::ffi::c_void,
        output_buffer_length: u32,
    ) -> i32;
}

#[cfg(windows)]
pub fn discover_windows_core_topology(core_count: usize) -> Vec<CoreMicroarch> {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
    };

    let mut buffer_len = 0u32;
    unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            std::ptr::null_mut(),
            &mut buffer_len,
        );
    }

    let mut microarch = vec![CoreMicroarch::LionCoveP; core_count];
    if buffer_len == 0 {
        for arch in microarch.iter_mut().take(core_count).skip(4) {
            *arch = CoreMicroarch::SkymontLpE;
        }
        return microarch;
    }

    let mut buffer = vec![0u8; buffer_len as usize];
    let ok = unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            buffer.as_mut_ptr() as *mut _,
            &mut buffer_len,
        )
    };

    if ok != 0 {
        let mut offset = 0;
        let mut core_idx = 0;
        while offset < buffer_len as usize && core_idx < core_count {
            let ptr = unsafe { buffer.as_ptr().add(offset) };
            let rel = unsafe { *(ptr as *const u32) };
            let size = unsafe { *(ptr.add(4) as *const u32) } as usize;
            if size == 0 {
                break;
            }
            if rel == RelationProcessorCore as u32 {
                let eff_class = unsafe { *ptr.add(9) };
                microarch[core_idx] = if eff_class == 0 {
                    CoreMicroarch::SkymontLpE
                } else {
                    CoreMicroarch::LionCoveP
                };
                core_idx += 1;
            }
            offset += size;
        }
    } else {
        for arch in microarch.iter_mut().take(core_count).skip(4) {
            *arch = CoreMicroarch::SkymontLpE;
        }
    }

    microarch
}

#[cfg(windows)]
fn collect_windows_cpu(state: &mut CpuCollectorState) -> LunarLakeCpuSnapshot {
    const CORE_COUNT: usize = 8;

    if state.topology.is_empty() {
        state.topology = discover_windows_core_topology(CORE_COUNT);
    }

    let mut perf_info = [SystemProcessorPerformanceInformation::default(); CORE_COUNT];
    let mut return_len: u32 = 0;

    let nt_status = unsafe {
        NtQuerySystemInformation(
            8, // SystemProcessorPerformanceInformation
            perf_info.as_mut_ptr() as *mut std::ffi::c_void,
            (std::mem::size_of::<SystemProcessorPerformanceInformation>() * CORE_COUNT) as u32,
            &mut return_len,
        )
    };

    let mut power_info = [ProcessorPowerInformation::default(); CORE_COUNT];
    let _ = unsafe {
        CallNtPowerInformation(
            11, // ProcessorInformation
            std::ptr::null(),
            0,
            power_info.as_mut_ptr() as *mut std::ffi::c_void,
            (std::mem::size_of::<ProcessorPowerInformation>() * CORE_COUNT) as u32,
        )
    };

    let mut per_core_pct = vec![0.0f32; CORE_COUNT];
    let mut per_core_current_mhz = vec![0u32; CORE_COUNT];
    let mut per_core_max_mhz = vec![0u32; CORE_COUNT];
    let mut per_core_limit_mhz = vec![0u32; CORE_COUNT];
    let mut per_core_effective_mhz = vec![0u32; CORE_COUNT];
    let mut is_throttled = false;

    for i in 0..CORE_COUNT {
        per_core_current_mhz[i] = power_info[i].current_mhz;
        per_core_max_mhz[i] = if power_info[i].max_mhz > 0 {
            power_info[i].max_mhz
        } else if state.topology.get(i) == Some(&CoreMicroarch::LionCoveP) {
            4800
        } else {
            3700
        };
        per_core_limit_mhz[i] = if power_info[i].mhz_limit > 0 {
            power_info[i].mhz_limit
        } else {
            per_core_max_mhz[i]
        };
        per_core_effective_mhz[i] = per_core_current_mhz[i];

        if per_core_limit_mhz[i] < per_core_max_mhz[i] {
            is_throttled = true;
        }
    }

    let mut total_user_delta = 0u64;
    let mut total_kernel_delta = 0u64;
    let mut total_idle_delta = 0u64;
    let mut total_dpc_delta = 0u64;
    let mut total_irq_delta = 0u64;

    let has_prev = state.prev_ticks.len() == CORE_COUNT;
    let mut next_ticks = Vec::with_capacity(CORE_COUNT);

    if nt_status == 0 {
        for (i, info) in perf_info.iter().enumerate().take(CORE_COUNT) {
            let u = info.user_time.max(0) as u64;
            let k = info.kernel_time.max(0) as u64;
            let idl = info.idle_time.max(0) as u64;
            let d = info.dpc_time.max(0) as u64;
            let irq = info.interrupt_time.max(0) as u64;

            next_ticks.push(CoreTickRecord {
                user: u,
                kernel: k,
                idle: idl,
                dpc: d,
                interrupt: irq,
            });

            if has_prev {
                let prev = state.prev_ticks[i];
                let u_diff = u.saturating_sub(prev.user);
                let k_diff = k.saturating_sub(prev.kernel);
                let idl_diff = idl.saturating_sub(prev.idle);
                let d_diff = d.saturating_sub(prev.dpc);
                let irq_diff = irq.saturating_sub(prev.interrupt);

                let sys_pure = k_diff.saturating_sub(idl_diff);
                let total_diff = u_diff + sys_pure + idl_diff;

                if total_diff > 0 {
                    let busy_diff = u_diff + sys_pure;
                    per_core_pct[i] = ((busy_diff as f64 / total_diff as f64) * 100.0) as f32;
                }

                total_user_delta += u_diff;
                total_kernel_delta += sys_pure;
                total_idle_delta += idl_diff;
                total_dpc_delta += d_diff;
                total_irq_delta += irq_diff;
            }
        }
    }

    state.prev_ticks = next_ticks;

    let grand_total = total_user_delta + total_kernel_delta + total_idle_delta;
    let (user_pct, system_pct, idle_pct, dpc_pct, interrupt_pct) = if grand_total > 0 {
        (
            (total_user_delta as f64 / grand_total as f64) * 100.0,
            (total_kernel_delta as f64 / grand_total as f64) * 100.0,
            (total_idle_delta as f64 / grand_total as f64) * 100.0,
            Some((total_dpc_delta as f64 / grand_total as f64) * 100.0),
            Some((total_irq_delta as f64 / grand_total as f64) * 100.0),
        )
    } else {
        (0.0, 0.0, 100.0, None, None)
    };

    let mut p_sum = 0.0f32;
    let mut p_count = 0usize;
    let mut lpe_sum = 0.0f32;
    let mut lpe_count = 0usize;

    for (i, &arch) in state.topology.iter().enumerate().take(CORE_COUNT) {
        if arch == CoreMicroarch::LionCoveP {
            p_sum += per_core_pct[i];
            p_count += 1;
        } else {
            lpe_sum += per_core_pct[i];
            lpe_count += 1;
        }
    }

    let lion_cove_p_load_pct = if p_count > 0 {
        p_sum / p_count as f32
    } else {
        0.0
    };
    let skymont_lpe_load_pct = if lpe_count > 0 {
        lpe_sum / lpe_count as f32
    } else {
        0.0
    };
    let total_load_pct = per_core_pct.iter().sum::<f32>() / CORE_COUNT as f32;

    LunarLakeCpuSnapshot {
        total_load_pct,
        lion_cove_p_load_pct,
        skymont_lpe_load_pct,
        per_core_pct,
        per_core_effective_mhz,
        per_core_current_mhz,
        per_core_max_mhz,
        per_core_limit_mhz,
        per_core_microarch: state.topology.clone(),
        is_throttled,
        user_pct,
        system_pct,
        idle_pct,
        iowait_pct: 0.0,
        dpc_pct,
        interrupt_pct,
    }
}

#[cfg(not(windows))]
fn collect_linux_cpu(_state: &mut CpuCollectorState) -> LunarLakeCpuSnapshot {
    mock_cpu_snapshot()
}
