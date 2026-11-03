// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::snapshot::types::UmaMemorySnapshot;

pub fn collect_memory_snapshot(
    arc_gpu_shared_mb: u64,
    npu_shared_mb: u64,
    mock: bool,
) -> UmaMemorySnapshot {
    if mock {
        return mock_memory_snapshot(arc_gpu_shared_mb, npu_shared_mb);
    }

    #[cfg(windows)]
    {
        collect_windows_memory(arc_gpu_shared_mb, npu_shared_mb)
    }

    #[cfg(not(windows))]
    {
        collect_linux_memory(arc_gpu_shared_mb, npu_shared_mb)
    }
}

fn mock_memory_snapshot(arc_gpu_shared_mb: u64, npu_shared_mb: u64) -> UmaMemorySnapshot {
    const INSTALLED_MOP_MB: u64 = 16384;
    let os_total_phys_mb: u64 = 15872;
    let bios_stolen_aperture_mb = INSTALLED_MOP_MB.saturating_sub(os_total_phys_mb);
    let os_avail_phys_mb: u64 = 10240;
    let in_use_phys_mb = os_total_phys_mb.saturating_sub(os_avail_phys_mb);
    let gpu_shared = if arc_gpu_shared_mb > 0 {
        arc_gpu_shared_mb
    } else {
        1024
    };
    let npu_shared = if npu_shared_mb > 0 {
        npu_shared_mb
    } else {
        128
    };
    let pure_cpu_in_use_mb = in_use_phys_mb.saturating_sub(gpu_shared + npu_shared);

    UmaMemorySnapshot {
        installed_mop_mb: INSTALLED_MOP_MB,
        bios_stolen_aperture_mb,
        os_total_phys_mb,
        os_avail_phys_mb,
        in_use_phys_mb,
        pure_cpu_in_use_mb,
        arc_gpu_shared_mb: gpu_shared,
        npu_shared_mb: npu_shared,
        commit_total_mb: 7200,
        commit_limit_mb: 19456,
        standby_mb: Some(4096),
        modified_mb: Some(512),
        free_zero_mb: Some(5632),
    }
}

#[cfg(windows)]
fn collect_windows_memory(arc_gpu_shared_mb: u64, npu_shared_mb: u64) -> UmaMemorySnapshot {
    use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    const INSTALLED_MOP_MB: u64 = 16384;
    let mut mem_status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    mem_status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;

    let (os_total_phys_mb, os_avail_phys_mb) = unsafe {
        if GlobalMemoryStatusEx(&mut mem_status) != 0 {
            (
                mem_status.ullTotalPhys / (1024 * 1024),
                mem_status.ullAvailPhys / (1024 * 1024),
            )
        } else {
            (15872, 8192)
        }
    };

    let bios_stolen_aperture_mb = INSTALLED_MOP_MB.saturating_sub(os_total_phys_mb);
    let in_use_phys_mb = os_total_phys_mb.saturating_sub(os_avail_phys_mb);
    let pure_cpu_in_use_mb = in_use_phys_mb.saturating_sub(arc_gpu_shared_mb + npu_shared_mb);

    let mut perf_info: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
    perf_info.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;

    let (commit_total_mb, commit_limit_mb) = unsafe {
        if GetPerformanceInfo(
            &mut perf_info,
            std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        ) != 0
        {
            let page_size_mb = perf_info.PageSize as u64 / (1024 * 1024);
            let page_size_kb = perf_info.PageSize as u64 / 1024;
            let (tot_mb, lim_mb) = if page_size_mb > 0 {
                (
                    perf_info.CommitTotal as u64 * page_size_mb,
                    perf_info.CommitLimit as u64 * page_size_mb,
                )
            } else {
                (
                    (perf_info.CommitTotal as u64 * page_size_kb) / 1024,
                    (perf_info.CommitLimit as u64 * page_size_kb) / 1024,
                )
            };
            (tot_mb, lim_mb)
        } else {
            (in_use_phys_mb, os_total_phys_mb + 2048)
        }
    };

    UmaMemorySnapshot {
        installed_mop_mb: INSTALLED_MOP_MB,
        bios_stolen_aperture_mb,
        os_total_phys_mb,
        os_avail_phys_mb,
        in_use_phys_mb,
        pure_cpu_in_use_mb,
        arc_gpu_shared_mb,
        npu_shared_mb,
        commit_total_mb,
        commit_limit_mb,
        standby_mb: None,
        modified_mb: None,
        free_zero_mb: None,
    }
}

#[cfg(not(windows))]
fn collect_linux_memory(arc_gpu_shared_mb: u64, npu_shared_mb: u64) -> UmaMemorySnapshot {
    const INSTALLED_MOP_MB: u64 = 16384;
    let mut mem_total_kb = 0u64;
    let mut mem_avail_kb = 0u64;

    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                mem_total_kb = parse_meminfo_kb(line);
            } else if line.starts_with("MemAvailable:") {
                mem_avail_kb = parse_meminfo_kb(line);
            }
        }
    }

    let os_total_phys_mb = if mem_total_kb > 0 {
        mem_total_kb / 1024
    } else {
        15872
    };
    let os_avail_phys_mb = if mem_avail_kb > 0 {
        mem_avail_kb / 1024
    } else {
        10240
    };
    let bios_stolen_aperture_mb = INSTALLED_MOP_MB.saturating_sub(os_total_phys_mb);
    let in_use_phys_mb = os_total_phys_mb.saturating_sub(os_avail_phys_mb);
    let pure_cpu_in_use_mb = in_use_phys_mb.saturating_sub(arc_gpu_shared_mb + npu_shared_mb);

    UmaMemorySnapshot {
        installed_mop_mb: INSTALLED_MOP_MB,
        bios_stolen_aperture_mb,
        os_total_phys_mb,
        os_avail_phys_mb,
        in_use_phys_mb,
        pure_cpu_in_use_mb,
        arc_gpu_shared_mb,
        npu_shared_mb,
        commit_total_mb: in_use_phys_mb,
        commit_limit_mb: os_total_phys_mb,
        standby_mb: None,
        modified_mb: None,
        free_zero_mb: None,
    }
}

#[cfg(not(windows))]
fn parse_meminfo_kb(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}
