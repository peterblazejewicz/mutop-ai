// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::hardware::pdh::win_pdh::PdhTelemetryState;
use crate::snapshot::types::{CoreMicroarch, LunarLakeThreadPlacement, ProcessAffinityProfile};
use std::collections::HashMap;

pub fn collect_tracked_processes(
    pdh_state: Option<&PdhTelemetryState>,
    patterns: &[String],
    include_threads: bool,
    mock: bool,
) -> Vec<ProcessAffinityProfile> {
    if mock {
        return mock_tracked_processes(include_threads);
    }
    if patterns.is_empty() {
        return Vec::new();
    }
    #[cfg(windows)]
    {
        collect_windows_processes(pdh_state, patterns, include_threads)
    }
    #[cfg(not(windows))]
    {
        let _ = pdh_state;
        let _ = patterns;
        mock_tracked_processes(include_threads)
    }
}

fn mock_tracked_processes(include_threads: bool) -> Vec<ProcessAffinityProfile> {
    let mut core_dist = HashMap::new();
    core_dist.insert("core0".to_string(), 2);
    core_dist.insert("core1".to_string(), 1);
    core_dist.insert("core2".to_string(), 1);
    core_dist.insert("core3".to_string(), 1);

    let threads = vec![
        LunarLakeThreadPlacement {
            tid: 1421,
            name: "llama-main".to_string(),
            cpu_core: 0,
            microarch: CoreMicroarch::LionCoveP,
            is_eco_qos: false,
        },
        LunarLakeThreadPlacement {
            tid: 1422,
            name: "llama-compute-0".to_string(),
            cpu_core: 1,
            microarch: CoreMicroarch::LionCoveP,
            is_eco_qos: false,
        },
        LunarLakeThreadPlacement {
            tid: 1423,
            name: "llama-compute-1".to_string(),
            cpu_core: 2,
            microarch: CoreMicroarch::LionCoveP,
            is_eco_qos: false,
        },
        LunarLakeThreadPlacement {
            tid: 1424,
            name: "llama-compute-2".to_string(),
            cpu_core: 3,
            microarch: CoreMicroarch::LionCoveP,
            is_eco_qos: false,
        },
        LunarLakeThreadPlacement {
            tid: 1425,
            name: "llama-http".to_string(),
            cpu_core: 0,
            microarch: CoreMicroarch::LionCoveP,
            is_eco_qos: false,
        },
    ];

    vec![ProcessAffinityProfile {
        pid: 1420,
        name: "llama-server.exe".to_string(),
        cmdline: "llama-server.exe -m /models/functiongemma-270m.gguf -ngl 99".to_string(),
        cpu_pct: 28.5,
        lion_cove_p_cpu_pct: 28.5,
        skymont_lpe_cpu_pct: 0.0,
        cpu_working_set_mb: 384,
        gpu_shared_mb: 768,
        npu_shared_mb: 0,
        commit_private_mb: 450,
        thread_count: 5,
        lion_cove_p_threads: 5,
        skymont_lpe_threads: 0,
        is_eco_qos: false,
        core_distribution: core_dist,
        threads: if include_threads { threads } else { Vec::new() },
    }]
}

#[cfg(windows)]
fn collect_windows_processes(
    pdh_state: Option<&PdhTelemetryState>,
    patterns: &[String],
    include_threads: bool,
) -> Vec<ProcessAffinityProfile> {
    use crate::hardware::cpu::discover_windows_core_topology;
    use crate::hardware::dxgi::win_dxgi::enumerate_dxgi_adapters;
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION,
        THREAD_QUERY_LIMITED_INFORMATION,
    };

    #[repr(C)]
    #[derive(Default)]
    struct ProcessorNumber {
        group: u16,
        number: u8,
        reserved: u8,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ThreadPowerThrottlingState {
        version: u32,
        control_flags: u32,
        state_mask: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetThreadIdealProcessorEx(
            h_thread: windows_sys::Win32::Foundation::HANDLE,
            lp_ideal_processor: *mut ProcessorNumber,
        ) -> i32;

        fn QueryThreadCycleTime(
            h_thread: windows_sys::Win32::Foundation::HANDLE,
            cycle_time: *mut u64,
        ) -> i32;

        fn GetThreadInformation(
            h_thread: windows_sys::Win32::Foundation::HANDLE,
            thread_information_class: u32,
            thread_information: *mut std::ffi::c_void,
            thread_information_size: u32,
        ) -> i32;
    }

    let mut sys = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let topology = discover_windows_core_topology(8);
    let adapters = enumerate_dxgi_adapters();
    let gpu_luid = adapters
        .iter()
        .find(|a| a.is_gpu)
        .map(|a| a.luid.pdh_pattern().to_lowercase());
    let npu_luid = adapters
        .iter()
        .find(|a| a.is_npu)
        .map(|a| a.luid.pdh_pattern().to_lowercase());

    let mut proc_shared_mem: HashMap<u32, (u64, u64)> = HashMap::new();
    if let Some(pdh) = pdh_state {
        let items = pdh.read_array("gpu_proc_mem");
        for (name, bytes) in items {
            let name_lower = name.to_lowercase();
            if let Some(pid) = extract_pid(&name_lower) {
                let mb = (bytes / (1024.0 * 1024.0)) as u64;
                let entry = proc_shared_mem.entry(pid).or_insert((0, 0));
                if let Some(ref gluid) = gpu_luid {
                    if name_lower.contains(gluid) {
                        entry.0 += mb;
                    }
                }
                if let Some(ref nluid) = npu_luid {
                    if name_lower.contains(nluid) {
                        entry.1 += mb;
                    }
                }
            }
        }
    }

    let mut results = Vec::new();

    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_string();
        let name_lower = name.to_lowercase();

        let matches = patterns
            .iter()
            .any(|pat| name_lower.contains(&pat.to_lowercase()));
        if !matches {
            continue;
        }

        let pid_u32 = pid.as_u32();
        let cmdline = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let total_cpu = proc.cpu_usage();

        let mut working_set_mb = proc.memory() / (1024 * 1024);
        let mut commit_mb = working_set_mb;

        let h_proc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid_u32) };
        if !h_proc.is_null() && h_proc != INVALID_HANDLE_VALUE {
            let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let ok = unsafe {
                K32GetProcessMemoryInfo(
                    h_proc,
                    &mut pmc,
                    std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                )
            };
            if ok != 0 {
                working_set_mb = pmc.WorkingSetSize as u64 / (1024 * 1024);
                commit_mb = pmc.PagefileUsage as u64 / (1024 * 1024);
            }
            unsafe { CloseHandle(h_proc) };
        }

        let mut p_cycles = 0u64;
        let mut lpe_cycles = 0u64;
        let mut p_threads = 0usize;
        let mut lpe_threads = 0usize;
        let mut thread_records = Vec::new();
        let mut core_dist: HashMap<String, usize> = HashMap::new();
        let mut has_eco_qos = false;

        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snap != INVALID_HANDLE_VALUE {
            let mut te: THREADENTRY32 = unsafe { std::mem::zeroed() };
            te.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

            if unsafe { Thread32First(snap, &mut te) } != 0 {
                loop {
                    if te.th32OwnerProcessID == pid_u32 {
                        let tid = te.th32ThreadID;
                        let h_thread =
                            unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid) };
                        if !h_thread.is_null() && h_thread != INVALID_HANDLE_VALUE {
                            let mut ideal_proc = ProcessorNumber::default();
                            let _ = unsafe { GetThreadIdealProcessorEx(h_thread, &mut ideal_proc) };
                            let core = ideal_proc.number as u32;

                            let microarch = if let Some(&arch) = topology.get(core as usize) {
                                arch
                            } else if core < 4 {
                                CoreMicroarch::LionCoveP
                            } else {
                                CoreMicroarch::SkymontLpE
                            };

                            let mut cycle_time: u64 = 0;
                            unsafe { QueryThreadCycleTime(h_thread, &mut cycle_time) };

                            if microarch == CoreMicroarch::LionCoveP {
                                p_cycles += cycle_time;
                                p_threads += 1;
                            } else {
                                lpe_cycles += cycle_time;
                                lpe_threads += 1;
                            }

                            let mut throttling = ThreadPowerThrottlingState {
                                version: 1,
                                control_flags: 0,
                                state_mask: 0,
                            };
                            let thread_eco_qos = unsafe {
                                GetThreadInformation(
                                    h_thread,
                                    1, // ThreadPowerThrottling
                                    &mut throttling as *mut _ as *mut std::ffi::c_void,
                                    std::mem::size_of::<ThreadPowerThrottlingState>() as u32,
                                ) != 0
                                    && (throttling.control_flags & 1) != 0
                            };
                            if thread_eco_qos {
                                has_eco_qos = true;
                            }

                            *core_dist.entry(format!("core{}", core)).or_insert(0) += 1;

                            thread_records.push(LunarLakeThreadPlacement {
                                tid,
                                name: format!("thread-{}", tid),
                                cpu_core: core,
                                microarch,
                                is_eco_qos: thread_eco_qos,
                            });

                            unsafe { CloseHandle(h_thread) };
                        }
                    }
                    if unsafe { Thread32Next(snap, &mut te) } == 0 {
                        break;
                    }
                }
            }
            unsafe { CloseHandle(snap) };
        }

        let total_cycles = p_cycles + lpe_cycles;
        let (lion_cove_p_cpu_pct, skymont_lpe_cpu_pct) = if total_cycles > 0 {
            (
                total_cpu * (p_cycles as f32 / total_cycles as f32),
                total_cpu * (lpe_cycles as f32 / total_cycles as f32),
            )
        } else if p_threads + lpe_threads > 0 {
            let tot = (p_threads + lpe_threads) as f32;
            (
                total_cpu * (p_threads as f32 / tot),
                total_cpu * (lpe_threads as f32 / tot),
            )
        } else {
            (total_cpu, 0.0)
        };

        let (gpu_shared_mb, npu_shared_mb) =
            proc_shared_mem.get(&pid_u32).copied().unwrap_or((0, 0));

        results.push(ProcessAffinityProfile {
            pid: pid_u32,
            name,
            cmdline,
            cpu_pct: total_cpu,
            lion_cove_p_cpu_pct,
            skymont_lpe_cpu_pct,
            cpu_working_set_mb: working_set_mb,
            gpu_shared_mb,
            npu_shared_mb,
            commit_private_mb: commit_mb,
            thread_count: thread_records.len().max(1),
            lion_cove_p_threads: p_threads,
            skymont_lpe_threads: lpe_threads,
            is_eco_qos: has_eco_qos,
            core_distribution: core_dist,
            threads: if include_threads {
                thread_records
            } else {
                Vec::new()
            },
        });
    }

    results
}

#[cfg(windows)]
fn extract_pid(name: &str) -> Option<u32> {
    if let Some(pos) = name.find("pid_") {
        let after = &name[pos + 4..];
        let end = after.find('_').unwrap_or(after.len());
        after[..end].parse::<u32>().ok()
    } else {
        None
    }
}
