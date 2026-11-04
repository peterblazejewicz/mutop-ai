// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::hardware::dxgi::AdapterInfo;
use crate::hardware::pdh::win_pdh::PdhTelemetryState;
use crate::snapshot::types::{CoreMicroarch, LunarLakeThreadPlacement, ProcessAffinityProfile};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Default)]
pub struct CachedThreadInfo {
    pub p_cycles: u64,
    pub lpe_cycles: u64,
    pub p_threads: usize,
    pub lpe_threads: usize,
    pub core_dist: HashMap<String, usize>,
    pub has_eco_qos: bool,
    pub threads: Vec<LunarLakeThreadPlacement>,
}

#[derive(Default)]
pub struct ProcessesCollectorState {
    pub prev_procs: HashMap<u32, ProcTimeSample>,
    pub last_proc_scan: Option<Instant>,
    pub cached_matched_pids: Vec<(u32, String)>,
    pub last_thread_poll: Option<Instant>,
    pub cached_thread_info: HashMap<u32, CachedThreadInfo>,
    pub cached_results: Vec<ProcessAffinityProfile>,
}

#[derive(Clone)]
pub struct ProcTimeSample {
    pub total_100ns: u64,
    pub timestamp: Instant,
    pub cmdline: String,
}

pub fn collect_tracked_processes(
    state: &mut ProcessesCollectorState,
    adapters: &[AdapterInfo],
    topology: &[CoreMicroarch],
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
        collect_windows_processes(
            state,
            adapters,
            topology,
            pdh_state,
            patterns,
            include_threads,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        let _ = adapters;
        let _ = topology;
        let _ = pdh_state;
        let _ = patterns;
        mock_tracked_processes(include_threads)
    }
}

fn mock_tracked_processes(include_threads: bool) -> Vec<ProcessAffinityProfile> {
    let mut core_dist1 = HashMap::new();
    core_dist1.insert("core0".to_string(), 2);
    core_dist1.insert("core1".to_string(), 1);
    core_dist1.insert("core2".to_string(), 1);
    core_dist1.insert("core3".to_string(), 1);

    let threads1 = vec![
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

    let mut core_dist2 = HashMap::new();
    core_dist2.insert("core5".to_string(), 1);
    core_dist2.insert("core6".to_string(), 1);

    let threads2 = vec![
        LunarLakeThreadPlacement {
            tid: 2829,
            name: "mutop-tui".to_string(),
            cpu_core: 5,
            microarch: CoreMicroarch::SkymontLpE,
            is_eco_qos: true,
        },
        LunarLakeThreadPlacement {
            tid: 2830,
            name: "mutop-sampler".to_string(),
            cpu_core: 6,
            microarch: CoreMicroarch::SkymontLpE,
            is_eco_qos: true,
        },
    ];

    vec![
        ProcessAffinityProfile {
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
            core_distribution: core_dist1,
            threads: if include_threads {
                threads1
            } else {
                Vec::new()
            },
        },
        ProcessAffinityProfile {
            pid: 2828,
            name: "mutop-ai.exe".to_string(),
            cmdline: "mutop-ai.exe".to_string(),
            cpu_pct: 0.4,
            lion_cove_p_cpu_pct: 0.0,
            skymont_lpe_cpu_pct: 0.4,
            cpu_working_set_mb: 14,
            gpu_shared_mb: 0,
            npu_shared_mb: 0,
            commit_private_mb: 8,
            thread_count: 2,
            lion_cove_p_threads: 0,
            skymont_lpe_threads: 2,
            is_eco_qos: true,
            core_distribution: core_dist2,
            threads: if include_threads {
                threads2
            } else {
                Vec::new()
            },
        },
        ProcessAffinityProfile {
            pid: 3310,
            name: "python.exe".to_string(),
            cmdline: "python train.py".to_string(),
            cpu_pct: 12.0,
            lion_cove_p_cpu_pct: 12.0,
            skymont_lpe_cpu_pct: 0.0,
            cpu_working_set_mb: 512,
            gpu_shared_mb: 256,
            npu_shared_mb: 0,
            commit_private_mb: 600,
            thread_count: 4,
            lion_cove_p_threads: 4,
            skymont_lpe_threads: 0,
            is_eco_qos: false,
            core_distribution: HashMap::new(),
            threads: Vec::new(),
        },
        ProcessAffinityProfile {
            pid: 4,
            name: "System".to_string(),
            cmdline: "System".to_string(),
            cpu_pct: 1.2,
            lion_cove_p_cpu_pct: 0.6,
            skymont_lpe_cpu_pct: 0.6,
            cpu_working_set_mb: 4,
            gpu_shared_mb: 0,
            npu_shared_mb: 0,
            commit_private_mb: 2,
            thread_count: 8,
            lion_cove_p_threads: 4,
            skymont_lpe_threads: 4,
            is_eco_qos: false,
            core_distribution: HashMap::new(),
            threads: Vec::new(),
        },
    ]
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ProcessorNumber {
    group: u16,
    number: u8,
    reserved: u8,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ThreadPowerThrottlingState {
    version: u32,
    control_flags: u32,
    state_mask: u32,
}

#[cfg(windows)]
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

#[cfg(windows)]
fn get_process_cmdline(
    h_proc: windows_sys::Win32::Foundation::HANDLE,
    fallback_name: &str,
) -> String {
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }

    extern "system" {
        fn NtQueryInformationProcess(
            process_handle: windows_sys::Win32::Foundation::HANDLE,
            process_information_class: u32,
            process_information: *mut std::ffi::c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    let mut return_len: u32 = 0;
    // ProcessCommandLineInformation = 60
    let _ =
        unsafe { NtQueryInformationProcess(h_proc, 60, std::ptr::null_mut(), 0, &mut return_len) };

    if return_len > std::mem::size_of::<UnicodeString>() as u32 {
        let mut buf = vec![0u8; return_len as usize];
        let status = unsafe {
            NtQueryInformationProcess(
                h_proc,
                60,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                return_len,
                &mut return_len,
            )
        };
        if status == 0 {
            let ustr = unsafe { &*(buf.as_ptr() as *const UnicodeString) };
            if !ustr.buffer.is_null() && ustr.length > 0 {
                let slice =
                    unsafe { std::slice::from_raw_parts(ustr.buffer, (ustr.length / 2) as usize) };
                return String::from_utf16_lossy(slice);
            }
        }
    }

    fallback_name.to_string()
}

#[cfg(windows)]
fn collect_windows_processes(
    state: &mut ProcessesCollectorState,
    adapters: &[AdapterInfo],
    topology: &[CoreMicroarch],
    pdh_state: Option<&PdhTelemetryState>,
    patterns: &[String],
    include_threads: bool,
) -> Vec<ProcessAffinityProfile> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Thread32First, Thread32Next,
        PROCESSENTRY32W, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION,
        THREAD_QUERY_LIMITED_INFORMATION,
    };

    let match_all = patterns.iter().any(|p| p == "*");

    let should_scan_threads = state
        .last_thread_poll
        .map(|t| t.elapsed() >= std::time::Duration::from_millis(1000))
        .unwrap_or(true);

    if match_all && !should_scan_threads && !state.cached_results.is_empty() {
        return state.cached_results.clone();
    }

    let gpu_luid = adapters
        .iter()
        .filter(|a| a.is_gpu)
        .find(|a| {
            let pat = a.luid.pdh_pattern().to_lowercase();
            pdh_state
                .map(|p| {
                    p.read_array("gpu_proc_mem")
                        .iter()
                        .any(|(name, _)| name.to_lowercase().contains(&pat))
                })
                .unwrap_or(false)
        })
        .or_else(|| adapters.iter().find(|a| a.is_gpu))
        .map(|a| a.luid.pdh_pattern().to_lowercase());

    let npu_luid = adapters
        .iter()
        .find(|a| a.is_npu)
        .map(|a| a.luid.pdh_pattern().to_lowercase())
        .or_else(|| {
            pdh_state.and_then(|pdh| {
                let engines = pdh.read_array("gpu_engines");
                engines.iter().find_map(|(name, _)| {
                    let nl = name.to_lowercase();
                    if nl.contains("engtype_neural") || nl.contains("engtype_npu") {
                        if let Some(pos) = nl.find("luid_0x") {
                            if nl.len() >= pos + 23 {
                                return Some(nl[pos..pos + 23].to_string());
                            }
                        }
                    }
                    None
                })
            })
        });

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

    let should_scan_pids = state
        .last_proc_scan
        .map(|t| t.elapsed() >= std::time::Duration::from_millis(1000))
        .unwrap_or(true);

    if should_scan_pids {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap != INVALID_HANDLE_VALUE {
            let mut pe: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
            pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            let mut matched = Vec::new();
            if unsafe { Process32FirstW(snap, &mut pe) } != 0 {
                loop {
                    let nul_pos = pe
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(pe.szExeFile.len());
                    let name = String::from_utf16_lossy(&pe.szExeFile[..nul_pos]);
                    let name_lower = name.to_lowercase();

                    if pe.th32ProcessID > 0
                        && (match_all
                            || patterns
                                .iter()
                                .any(|pat| name_lower.contains(&pat.to_lowercase())))
                    {
                        matched.push((pe.th32ProcessID, name));
                    }

                    if unsafe { Process32NextW(snap, &mut pe) } == 0 {
                        break;
                    }
                }
            }
            unsafe { CloseHandle(snap) };
            state.cached_matched_pids = matched;
            state.last_proc_scan = Some(Instant::now());
        }
    }

    if should_scan_threads {
        state.last_thread_poll = Some(Instant::now());
        if !state.cached_matched_pids.is_empty() {
            let mut target_thread_pids: std::collections::HashSet<u32> =
                std::collections::HashSet::new();
            if match_all {
                for (pid, name) in &state.cached_matched_pids {
                    let nl = name.to_lowercase();
                    if nl.contains("llama")
                        || nl.contains("python")
                        || nl.contains("mutop")
                        || nl.contains("device-commander")
                        || nl.contains("qnn")
                        || proc_shared_mem.contains_key(pid)
                    {
                        target_thread_pids.insert(*pid);
                    }
                }
                let mut top_pids: Vec<(u32, u64)> = state
                    .prev_procs
                    .iter()
                    .map(|(&pid, s)| (pid, s.total_100ns))
                    .collect();
                top_pids.sort_by(|a, b| b.1.cmp(&a.1));
                for (pid, _) in top_pids.into_iter().take(25) {
                    target_thread_pids.insert(pid);
                }
            }

            let snap_th = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if snap_th != INVALID_HANDLE_VALUE {
                let mut pid_to_threads: HashMap<u32, Vec<u32>> = HashMap::new();
                let mut te: THREADENTRY32 = unsafe { std::mem::zeroed() };
                te.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

                if unsafe { Thread32First(snap_th, &mut te) } != 0 {
                    loop {
                        let should_track = if match_all {
                            target_thread_pids.contains(&te.th32OwnerProcessID)
                        } else {
                            state
                                .cached_matched_pids
                                .iter()
                                .any(|(p, _)| *p == te.th32OwnerProcessID)
                        };
                        if should_track {
                            pid_to_threads
                                .entry(te.th32OwnerProcessID)
                                .or_default()
                                .push(te.th32ThreadID);
                        }
                        if unsafe { Thread32Next(snap_th, &mut te) } == 0 {
                            break;
                        }
                    }
                }
                unsafe { CloseHandle(snap_th) };

                for (&pid_u32, tids) in &pid_to_threads {
                    let mut info = CachedThreadInfo::default();
                    for &tid in tids {
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
                                info.p_cycles += cycle_time;
                                info.p_threads += 1;
                            } else {
                                info.lpe_cycles += cycle_time;
                                info.lpe_threads += 1;
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
                                info.has_eco_qos = true;
                            }

                            *info.core_dist.entry(format!("core{}", core)).or_insert(0) += 1;

                            if include_threads {
                                info.threads.push(LunarLakeThreadPlacement {
                                    tid,
                                    name: format!("thread-{}", tid),
                                    cpu_core: core,
                                    microarch,
                                    is_eco_qos: thread_eco_qos,
                                });
                            }

                            unsafe { CloseHandle(h_thread) };
                        }
                    }
                    state.cached_thread_info.insert(pid_u32, info);
                }
            }
        }
    }

    let now = Instant::now();
    let mut new_procs: HashMap<u32, ProcTimeSample> = HashMap::new();
    let mut results = Vec::new();

    let matched_pids = state.cached_matched_pids.clone();

    for (pid_u32, name) in matched_pids {
        let h_proc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid_u32) };
        if h_proc.is_null() || h_proc == INVALID_HANDLE_VALUE {
            continue;
        }

        let mut creation: FILETIME = unsafe { std::mem::zeroed() };
        let mut exit: FILETIME = unsafe { std::mem::zeroed() };
        let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
        let mut user: FILETIME = unsafe { std::mem::zeroed() };

        let mut total_cpu = 0.0f32;
        let mut cmdline = name.clone();

        if unsafe { GetProcessTimes(h_proc, &mut creation, &mut exit, &mut kernel, &mut user) } != 0
        {
            let k = ((kernel.dwHighDateTime as u64) << 32) | (kernel.dwLowDateTime as u64);
            let u = ((user.dwHighDateTime as u64) << 32) | (user.dwLowDateTime as u64);
            let current_total_100ns = k + u;

            if let Some(prev) = state.prev_procs.get(&pid_u32) {
                let dt_sec = now.duration_since(prev.timestamp).as_secs_f64();
                let delta_100ns = current_total_100ns.saturating_sub(prev.total_100ns);
                let cpu_sec = (delta_100ns as f64) / 10_000_000.0;
                total_cpu = if dt_sec > 0.0 {
                    ((cpu_sec / (dt_sec * 8.0)) * 100.0).clamp(0.0, 100.0) as f32
                } else {
                    0.0
                };
                cmdline = prev.cmdline.clone();
            } else {
                cmdline = get_process_cmdline(h_proc, &name);
            }

            new_procs.insert(
                pid_u32,
                ProcTimeSample {
                    total_100ns: current_total_100ns,
                    timestamp: now,
                    cmdline: cmdline.clone(),
                },
            );
        }

        let mut working_set_mb = 0u64;
        let mut commit_mb = 0u64;
        let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if unsafe { K32GetProcessMemoryInfo(h_proc, &mut pmc, pmc.cb) } != 0 {
            working_set_mb = pmc.WorkingSetSize as u64 / (1024 * 1024);
            commit_mb = pmc.PagefileUsage as u64 / (1024 * 1024);
        }
        unsafe { CloseHandle(h_proc) };

        let thread_info = state
            .cached_thread_info
            .get(&pid_u32)
            .cloned()
            .unwrap_or_default();

        let total_cycles = thread_info.p_cycles + thread_info.lpe_cycles;
        let (lion_cove_p_cpu_pct, skymont_lpe_cpu_pct) = if total_cycles > 0 {
            (
                total_cpu * (thread_info.p_cycles as f32 / total_cycles as f32),
                total_cpu * (thread_info.lpe_cycles as f32 / total_cycles as f32),
            )
        } else if thread_info.p_threads + thread_info.lpe_threads > 0 {
            let tot = (thread_info.p_threads + thread_info.lpe_threads) as f32;
            (
                total_cpu * (thread_info.p_threads as f32 / tot),
                total_cpu * (thread_info.lpe_threads as f32 / tot),
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
            thread_count: (thread_info.p_threads + thread_info.lpe_threads).max(1),
            lion_cove_p_threads: thread_info.p_threads,
            skymont_lpe_threads: thread_info.lpe_threads,
            is_eco_qos: thread_info.has_eco_qos,
            core_distribution: thread_info.core_dist,
            threads: thread_info.threads,
        });
    }

    state.prev_procs = new_procs;
    state.cached_results = results.clone();
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
