// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::hardware::dxgi::AdapterInfo;
use crate::hardware::pdh::win_pdh::PdhTelemetryState;
use crate::snapshot::types::{
    AcceleratorProcess, IntelNpuSnapshot, LlamaSlotsMetrics, NpuActivityStatus,
};

pub fn collect_npu_snapshot(
    adapters: &[AdapterInfo],
    engine_items: &[(String, f64)],
    pdh_state: Option<&PdhTelemetryState>,
    mock: bool,
) -> IntelNpuSnapshot {
    if mock {
        return mock_npu_snapshot();
    }
    #[cfg(windows)]
    {
        collect_windows_npu(adapters, engine_items, pdh_state)
    }
    #[cfg(not(windows))]
    {
        let _ = adapters;
        let _ = engine_items;
        let _ = pdh_state;
        collect_linux_npu()
    }
}

fn mock_npu_snapshot() -> IntelNpuSnapshot {
    IntelNpuSnapshot {
        name: "Intel(R) AI Boost".to_string(),
        model: "NPU 4 (PCI 0x8086:0x643E)".to_string(),
        npu_tops: 47,
        adapter_luid: Some(0x1004),
        utilization_pct: 0.0,
        current_freq_mhz: 0,
        shared_memory_used_mb: 128,
        shared_memory_budget_mb: 15360,
        activity_status: NpuActivityStatus::Idle,
        slots_metrics: Some(LlamaSlotsMetrics {
            prompt_tokens_per_sec: 0.0,
            generation_tokens_per_sec: 0.0,
            active_slots: 0,
            total_slots: 4,
            prompt_tokens_processed: 0,
            tokens_decoded: 0,
            completed_task_tokens: None,
        }),
        active_processes: vec![AcceleratorProcess {
            pid: 1420,
            name: "llama-server.exe".to_string(),
            engine: "NPU 4 (Intel AI Boost)".to_string(),
            shared_memory_mb: 128,
            utilization_pct: 0.0,
        }],
    }
}

#[cfg(windows)]
fn collect_windows_npu(
    adapters: &[AdapterInfo],
    engine_items: &[(String, f64)],
    pdh_state: Option<&PdhTelemetryState>,
) -> IntelNpuSnapshot {
    use std::collections::HashSet;

    let chosen_npu = adapters.iter().find(|a| a.is_npu).cloned();

    // Fallback to PDH LUID deduction if hardware adapter wasn't enumerated by COM
    let mut deduced_luid_pattern = None;
    let mut deduced_luid_u64 = None;

    if chosen_npu.is_none() {
        let gpu_luids: HashSet<String> = adapters
            .iter()
            .filter(|a| a.is_gpu)
            .map(|a| a.luid.pdh_pattern().to_lowercase())
            .collect();

        for (name, _) in engine_items {
            let name_lower = name.to_lowercase();
            if name_lower.contains("engtype_compute")
                || name_lower.contains("engtype_npu")
                || name_lower.contains("engtype_neural")
            {
                if let Some(luid_pat) = extract_luid_pattern(&name_lower) {
                    if !gpu_luids.contains(&luid_pat) && !luid_pat.contains("0x00000000_0x00000000")
                    {
                        deduced_luid_u64 = parse_luid_from_pattern(&luid_pat);
                        deduced_luid_pattern = Some(luid_pat);
                        break;
                    }
                }
            }
        }
    }

    let (name, model, adapter_luid, luid_pattern, budget_mb) = match chosen_npu {
        Some(adapter) => {
            let luid_val = adapter.luid.to_u64();
            let pat = adapter.luid.pdh_pattern().to_lowercase();
            (
                adapter.description,
                format!(
                    "NPU 4 (PCI 0x{:04X}:0x{:04X})",
                    adapter.vendor_id, adapter.device_id
                ),
                Some(luid_val),
                Some(pat),
                adapter.shared_sys_mem_mb,
            )
        }
        None => match (deduced_luid_u64, deduced_luid_pattern) {
            (Some(luid_val), Some(pat)) => (
                "Intel(R) AI Boost".to_string(),
                "NPU 4 (PCI 0x8086:0x643E)".to_string(),
                Some(luid_val),
                Some(pat),
                0,
            ),
            _ => (
                "Intel(R) AI Boost".to_string(),
                "NPU 4 (PCI 0x8086:0x643E)".to_string(),
                None,
                None,
                0,
            ),
        },
    };

    let gpu_shared_budget = adapters
        .iter()
        .find(|a| a.is_gpu)
        .map(|a| a.shared_sys_mem_mb)
        .filter(|&m| m > 0)
        .unwrap_or(9156);
    let budget_mb = if budget_mb == 0 {
        gpu_shared_budget
    } else {
        budget_mb
    };

    let mut utilization_pct = 0.0f32;
    let mut shared_memory_used_mb = 0u64;
    let mut active_processes = Vec::new();

    if let Some(ref pat) = luid_pattern {
        for (inst_name, val) in engine_items {
            let inst_lower = inst_name.to_lowercase();
            if inst_lower.contains(pat) {
                let v = *val as f32;
                utilization_pct += v;
                if let Some(pid) = extract_pid_from_instance(&inst_lower) {
                    if !active_processes
                        .iter()
                        .any(|p: &AcceleratorProcess| p.pid == pid)
                    {
                        active_processes.push(AcceleratorProcess {
                            pid,
                            name: format!("PID {}", pid),
                            engine: "NPU Compute".to_string(),
                            shared_memory_mb: 0,
                            utilization_pct: v,
                        });
                    }
                }
            }
        }

        if let Some(pdh) = pdh_state {
            let adapter_mem = pdh.read_array("gpu_adapter_mem");
            for (name, bytes) in &adapter_mem {
                let name_lower = name.to_lowercase();
                if name_lower.contains(pat) {
                    let mb = (bytes / (1024.0 * 1024.0)) as u64;
                    if mb > 0 {
                        shared_memory_used_mb = mb;
                    }
                }
            }

            let mem_items = pdh.read_array("gpu_proc_mem");
            let mut proc_sum_mb = 0u64;
            for (name, bytes) in mem_items {
                let name_lower = name.to_lowercase();
                if name_lower.contains(pat) {
                    let mb = (bytes / (1024.0 * 1024.0)) as u64;
                    proc_sum_mb += mb;
                    if let Some(pid) = extract_pid_from_instance(&name_lower) {
                        if let Some(proc) = active_processes.iter_mut().find(|p| p.pid == pid) {
                            proc.shared_memory_mb = mb;
                        }
                    }
                }
            }
            if shared_memory_used_mb == 0 {
                shared_memory_used_mb = proc_sum_mb;
            }
        }
    }

    let activity_status = if utilization_pct > 1.0 {
        NpuActivityStatus::ActiveCompute
    } else {
        NpuActivityStatus::Idle
    };

    IntelNpuSnapshot {
        name,
        model,
        npu_tops: 47,
        adapter_luid,
        utilization_pct,
        current_freq_mhz: 0,
        shared_memory_used_mb,
        shared_memory_budget_mb: budget_mb,
        activity_status,
        slots_metrics: None,
        active_processes,
    }
}

#[cfg(windows)]
fn extract_luid_pattern(name: &str) -> Option<String> {
    if let Some(pos) = name.find("luid_0x") {
        let after = &name[pos..];
        // pattern: luid_0x<high>_0x<low>
        let parts: Vec<&str> = after.split('_').take(3).collect();
        if parts.len() == 3 {
            return Some(format!("{}_{}_{}", parts[0], parts[1], parts[2]));
        }
    }
    None
}

#[cfg(windows)]
fn parse_luid_from_pattern(pattern: &str) -> Option<u64> {
    // pattern: luid_0x12345678_0xabcdef01
    let parts: Vec<&str> = pattern.split('_').collect();
    if parts.len() >= 3 {
        let high_str = parts[1].strip_prefix("0x")?;
        let low_str = parts[2].strip_prefix("0x")?;
        let high = u32::from_str_radix(high_str, 16).ok()?;
        let low = u32::from_str_radix(low_str, 16).ok()?;
        Some(((high as u64) << 32) | (low as u64))
    } else {
        None
    }
}

#[cfg(windows)]
fn extract_pid_from_instance(name: &str) -> Option<u32> {
    if let Some(pos) = name.find("pid_") {
        let after = &name[pos + 4..];
        let end = after.find('_').unwrap_or(after.len());
        after[..end].parse::<u32>().ok()
    } else {
        None
    }
}

#[cfg(not(windows))]
fn collect_linux_npu() -> IntelNpuSnapshot {
    let accel_dir = std::path::Path::new("/sys/class/accel/accel0/device");
    if accel_dir.exists() {
        let freq_str = std::fs::read_to_string(accel_dir.join("freq/current_freq"))
            .unwrap_or_else(|_| "0".to_string());
        let current_freq_mhz = freq_str.trim().parse::<u32>().unwrap_or(0);
        let mem_str = std::fs::read_to_string(accel_dir.join("npu_memory_utilization"))
            .unwrap_or_else(|_| "0".to_string());
        let mem_bytes = mem_str.trim().parse::<u64>().unwrap_or(0);
        let shared_memory_used_mb = mem_bytes / (1024 * 1024);

        IntelNpuSnapshot {
            name: "Intel(R) AI Boost".to_string(),
            model: "NPU 4 (PCI 0x8086:0x643E)".to_string(),
            npu_tops: 47,
            adapter_luid: None,
            utilization_pct: if current_freq_mhz > 0 { 15.0 } else { 0.0 },
            current_freq_mhz,
            shared_memory_used_mb,
            shared_memory_budget_mb: 15360,
            activity_status: if current_freq_mhz > 0 {
                NpuActivityStatus::ActiveCompute
            } else {
                NpuActivityStatus::Idle
            },
            slots_metrics: None,
            active_processes: vec![],
        }
    } else {
        mock_npu_snapshot()
    }
}
