// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::snapshot::types::{
    AiDiagnostics, DiagnosticStatus, EffectivePowerMode, IntelArcGpuSnapshot, LunarLakeCpuSnapshot,
    NvmeStorageSnapshot, PowerThermalSnapshot, ProcessAffinityProfile, SystemTimingSnapshot,
    UmaMemorySnapshot,
};

pub fn evaluate_diagnostics(
    cpu: &LunarLakeCpuSnapshot,
    memory: &UmaMemorySnapshot,
    gpu: &IntelArcGpuSnapshot,
    power: &PowerThermalSnapshot,
    storage: &NvmeStorageSnapshot,
    timing: &SystemTimingSnapshot,
    processes: Option<&[ProcessAffinityProfile]>,
    is_marker_active: bool,
) -> AiDiagnostics {
    let mut bottlenecks = Vec::new();
    let mut recommendations = Vec::new();

    // 1. LLM worker on Skymont LP-E cores
    let mut llm_worker_on_skymont_lpe_cores = false;
    if let Some(procs) = processes {
        for p in procs {
            let name_lower = p.name.to_lowercase();
            if (name_lower.contains("llama") || name_lower.contains("whisper"))
                && (p.skymont_lpe_threads > 0 || p.skymont_lpe_cpu_pct > 5.0)
            {
                llm_worker_on_skymont_lpe_cores = true;
                bottlenecks.push(format!(
                    "Process '{}' (PID {}) running on Skymont LP-E cores ({} threads)",
                    p.name, p.pid, p.skymont_lpe_threads
                ));
                recommendations.push(
                    "Pin LLM/VLM worker threads to Lion Cove P-cores (affinity mask 0x0F) and disable Windows EcoQoS."
                        .to_string(),
                );
                break;
            }
        }
    }

    // 2. Timer resolution coarse during voice turn / active marker
    let timer_resolution_coarse = timing.timer_resolution.current_ms > 2.0;
    let timer_resolution_coarse_during_voice_turn = timer_resolution_coarse && is_marker_active;
    if timer_resolution_coarse_during_voice_turn {
        bottlenecks.push(format!(
            "OS timer resolution is coarse ({:.2} ms) during active voice/benchmark turn",
            timing.timer_resolution.current_ms
        ));
        recommendations.push(
            "Request 1.0 ms timer resolution via timeBeginPeriod(1) to avoid audio buffer jitter."
                .to_string(),
        );
    }

    // 3. UMA memory pressure near budget (> 85% of 16384 MB = 13926 MB)
    let uma_memory_pressure_near_budget = memory.in_use_phys_mb > 13926;
    if uma_memory_pressure_near_budget {
        bottlenecks.push(format!(
            "UMA physical memory pressure high: {} MB / {} MB used ({:.1}%)",
            memory.in_use_phys_mb,
            memory.installed_mop_mb,
            (memory.in_use_phys_mb as f64 / memory.installed_mop_mb as f64) * 100.0
        ));
        recommendations.push(
            "Reduce model context length or quantize weights to stay comfortably within 16 GB MoP."
                .to_string(),
        );
    }

    // 4. GPU or CPU power throttling
    // On integrated GPUs (e.g. Lunar Lake Xe2 Arc 140V), IGCL reports power, current,
    // and voltage limiters as active (1) during idle P-states due to low utilization (gpuUtilizationLimited).
    // To avoid false positives on idle hardware, non-thermal GPU limiters are only treated as active throttling
    // when the GPU is actively running a workload (total utilization >= 10.0%).
    let gpu_active = gpu.total_utilization_pct >= 10.0
        || gpu.render_3d_pct >= 5.0
        || gpu.compute_pct >= 5.0;
    let gpu_throttled = gpu.throttling.temp_limited
        || (gpu_active
            && (gpu.throttling.power_limited
                || gpu.throttling.current_limited
                || gpu.throttling.voltage_limited));
    let power_mode_throttled = power.effective_power_mode
        == EffectivePowerMode::BestPowerEfficiency
        || power.effective_power_mode == EffectivePowerMode::BatterySaver;
    let gpu_or_cpu_power_throttling = cpu.is_throttled || gpu_throttled || power_mode_throttled;

    if gpu_or_cpu_power_throttling {
        let mut reasons = Vec::new();
        if cpu.is_throttled {
            reasons.push("CPU clock limit < max");
        }
        if gpu_throttled {
            reasons.push("GPU limiter active");
        }
        if power_mode_throttled {
            reasons.push("Power mode throttled");
        }
        bottlenecks.push(format!(
            "Hardware power/thermal throttling detected ({})",
            reasons.join(", ")
        ));
        recommendations.push(
            "Set Windows 11 Power Mode to 'Best Performance' and verify active blower fan operation."
                .to_string(),
        );
    }

    // 5. NVMe PCIe link downgraded
    let nvme_pcie_link_downgraded = storage.is_link_degraded;
    if nvme_pcie_link_downgraded {
        bottlenecks.push(format!(
            "NVMe PCIe link degraded: running at Gen{} x{} (capable of Gen{} x{})",
            storage.current_gen, storage.current_lanes, storage.max_gen, storage.max_lanes
        ));
        recommendations.push(
            "Reseat M.2 2280 NVMe SSD in carrier board slot; link negotiated below PCIe Gen4 x4."
                .to_string(),
        );
    }

    // 6. NVMe thermal throttling
    let nvme_thermal_throttling = if let Some(ref smart) = storage.smart_health {
        smart.warning_throttle_minutes > 0 || smart.critical_throttle_minutes > 0
    } else {
        false
    };
    if nvme_thermal_throttling {
        bottlenecks.push("NVMe SMART log reports active thermal throttling".to_string());
        recommendations.push(
            "NVMe SSD temperature exceeds throttling threshold; check thermal pad and heatsink contact."
                .to_string(),
        );
    }

    let status = if nvme_thermal_throttling || (bottlenecks.len() >= 3) {
        DiagnosticStatus::Critical
    } else if !bottlenecks.is_empty() {
        DiagnosticStatus::Degraded
    } else {
        DiagnosticStatus::Healthy
    };

    AiDiagnostics {
        status,
        llm_worker_on_skymont_lpe_cores,
        timer_resolution_coarse_during_voice_turn,
        uma_memory_pressure_near_budget,
        gpu_or_cpu_power_throttling,
        nvme_pcie_link_downgraded,
        nvme_thermal_throttling,
        bottlenecks,
        recommendations,
    }
}
