// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use edgetop_core::inference::{parse_llm_args_from_cmdline, query_llama_slots};
use edgetop_core::{
    AcceleratorPhase, AiDiagnostics, EdgePlatform, PlatformMetadata, SchemaField, SummarySample,
    TickContext,
};

use crate::hardware::HardwareCollector;
use crate::snapshot::schema::{get_schema_json, SCHEMA_FIELDS};
use crate::snapshot::types::{
    DiagnosticStatus, LlamaSlotsMetrics, MutopSnapshot, NpuActivityStatus,
};

pub struct MutopPlatform {
    meta: PlatformMetadata,
    collector: HardwareCollector,
    pub sticky_llama_port: Option<u16>,
}

impl MutopPlatform {
    pub fn new() -> Self {
        Self {
            meta: PlatformMetadata {
                binary_name: "mutop-ai",
                board_name: "LattePanda Mu Ultra (Intel Core Ultra 7 256V)",
                description: "LattePanda Mu Ultra & Intel Lunar Lake Hardware Telemetry Monitor",
                version: env!("CARGO_PKG_VERSION"),
                default_idle_baseline_w: 8.5,
                default_stream_interval_ms: 200,
                mcp_server_name: "mutop-ai",
                mcp_scheme: "mutop",
                default_mark_file: "mutop-stream.mark",
                accelerator_name: "Intel AI Boost NPU & Arc 140V GPU",
            },
            collector: HardwareCollector::new(),
            sticky_llama_port: None,
        }
    }

    pub fn new_mock() -> Self {
        Self {
            meta: PlatformMetadata {
                binary_name: "mutop-ai",
                board_name: "LattePanda Mu Ultra (Intel Core Ultra 7 256V)",
                description: "LattePanda Mu Ultra & Intel Lunar Lake Hardware Telemetry Monitor",
                version: env!("CARGO_PKG_VERSION"),
                default_idle_baseline_w: 8.5,
                default_stream_interval_ms: 200,
                mcp_server_name: "mutop-ai",
                mcp_scheme: "mutop",
                default_mark_file: "mutop-stream.mark",
                accelerator_name: "Intel AI Boost NPU & Arc 140V GPU",
            },
            collector: HardwareCollector::new_mock(),
            sticky_llama_port: None,
        }
    }

    pub fn set_idle_baseline_w(&mut self, baseline: f64) {
        self.collector.idle_baseline_w = baseline;
    }

    pub fn calibrate_idle_baseline(
        &mut self,
        samples: usize,
        interval: std::time::Duration,
    ) -> Option<f64> {
        if samples == 0 {
            return None;
        }

        let mut sum = 0.0;
        let mut valid_samples = 0;
        let ctx = TickContext::default();

        for i in 0..samples {
            if i > 0 && !interval.is_zero() {
                std::thread::sleep(interval);
            }
            let snap = self.collect_snapshot(&ctx);
            let pkg = snap.power_thermals.package_w;
            if pkg > 0.0 {
                sum += pkg;
                valid_samples += 1;
            }
        }

        if valid_samples > 0 {
            let avg = (sum / valid_samples as f64 * 100.0).round() / 100.0;
            self.set_idle_baseline_w(avg);
            Some(avg)
        } else {
            None
        }
    }

    pub fn collector_mut(&mut self) -> &mut HardwareCollector {
        &mut self.collector
    }
}

impl Default for MutopPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl EdgePlatform for MutopPlatform {
    type Snapshot = MutopSnapshot;

    fn metadata(&self) -> &PlatformMetadata {
        &self.meta
    }

    fn collect_snapshot(&mut self, ctx: &TickContext) -> Self::Snapshot {
        let mut snapshot = self.collector.collect_snapshot(
            ctx.marker.clone(),
            &ctx.tracked_patterns,
            ctx.include_threads,
        );

        // Populate lifecycle events from tick context
        snapshot.events = ctx.events.clone();

        // Check llama-server inference slots if listening
        if snapshot.network.loopback.llama_server_listening {
            // Extract port from tracked llama-server process cmdline if available
            let port = if let Some(ref procs) = snapshot.tracked_processes {
                let mut found_port = None;
                for p in procs {
                    if p.name.contains("llama") {
                        let (_, _, _, _, _, _, _, _, p_port) =
                            parse_llm_args_from_cmdline(&p.cmdline);
                        if let Some(port) = p_port {
                            found_port = Some(port);
                            break;
                        }
                    }
                }
                found_port.or(self.sticky_llama_port).unwrap_or(8080)
            } else {
                self.sticky_llama_port.unwrap_or(8080)
            };
            self.sticky_llama_port = Some(port);

            if let Some(metrics) = query_llama_slots(port) {
                // Populate metrics on NPU snapshot
                snapshot.npu.slots_metrics = Some(LlamaSlotsMetrics {
                    prompt_tokens_per_sec: metrics.prompt_tokens_per_sec as f64,
                    generation_tokens_per_sec: metrics.decode_tokens_per_sec as f64,
                    active_slots: if metrics.slot_state == "idle" { 0 } else { 1 },
                    total_slots: 1,
                    prompt_tokens_processed: metrics.prompt_tokens_processed,
                    tokens_decoded: metrics.tokens_decoded,
                    completed_task_tokens: metrics.completed_task_tokens,
                });

                if metrics.slot_state == "processing_prompt" {
                    snapshot.npu.activity_status = NpuActivityStatus::ActivePromptProcessing;
                } else if metrics.slot_state == "generating_tokens" {
                    snapshot.npu.activity_status = NpuActivityStatus::ActiveTokenGeneration;
                }
            }
        }

        snapshot
    }

    fn extract_summary_sample(
        &self,
        snapshot: &Self::Snapshot,
        active_threshold_pct: f32,
    ) -> SummarySample {
        let interval_secs = snapshot.interval_ms.unwrap_or(200.0) / 1000.0;
        let power_w = Some(snapshot.power_thermals.package_w);
        let fan_rpm = snapshot.power_thermals.cpu_fan_rpm;
        let cpu_total_pct = snapshot.cpu.total_load_pct;
        let cpu_big_pct = snapshot.cpu.lion_cove_p_load_pct;
        let cpu_little_pct = snapshot.cpu.skymont_lpe_load_pct;
        let gpu_duty_pct = Some(snapshot.gpu.total_utilization_pct);
        let ram_available_gb = snapshot.memory.os_avail_phys_mb as f64 / 1024.0;
        let disk_read_mb_s = snapshot.storage.read_mb_s;
        let disk_write_mb_s = snapshot.storage.write_mb_s;
        let disk_util_pct = snapshot.storage.busy_pct;

        let total_rx_mbps: f64 = snapshot.network.interfaces.iter().map(|i| i.rx_mbps).sum();
        let total_tx_mbps: f64 = snapshot.network.interfaces.iter().map(|i| i.tx_mbps).sum();
        let net_rx_mb_s = total_rx_mbps / 8.0;
        let net_tx_mb_s = total_tx_mbps / 8.0;

        let mut temps = Vec::new();
        if let Some(t) = snapshot
            .power_thermals
            .cpu_package_dts_c
            .or(snapshot.power_thermals.acpi_thermal_zone_c)
        {
            temps.push(("cpu".to_string(), t));
        }
        if let Some(t) = snapshot.power_thermals.gpu_c {
            temps.push(("gpu".to_string(), t));
        }
        if let Some(t) = snapshot.power_thermals.nvme_composite_c {
            temps.push(("nvme".to_string(), t));
        }

        let accelerator_active = snapshot.npu.utilization_pct >= active_threshold_pct
            || snapshot.gpu.compute_pct >= active_threshold_pct
            || snapshot.npu.activity_status != NpuActivityStatus::Idle;

        let (
            prompt_tokens_processed,
            tokens_decoded,
            completed_task_tokens,
            prompt_tps,
            decode_tps,
        ) = if let Some(ref slots) = snapshot.npu.slots_metrics {
            (
                slots.prompt_tokens_processed,
                slots.tokens_decoded,
                slots.completed_task_tokens,
                Some(slots.prompt_tokens_per_sec as f32),
                Some(slots.generation_tokens_per_sec as f32),
            )
        } else {
            (0, 0, None, None, None)
        };

        let accelerator_phase = match snapshot.npu.activity_status {
            NpuActivityStatus::ActivePromptProcessing => AcceleratorPhase::PromptProcessing,
            NpuActivityStatus::ActiveTokenGeneration => AcceleratorPhase::TokenGeneration,
            NpuActivityStatus::ActiveCompute => AcceleratorPhase::OtherActive,
            NpuActivityStatus::Idle => {
                if accelerator_active {
                    AcceleratorPhase::OtherActive
                } else {
                    AcceleratorPhase::Idle
                }
            }
        };

        let tracked_proc_busy = snapshot
            .tracked_processes
            .as_ref()
            .map(|procs| procs.iter().any(|p| p.cpu_pct >= active_threshold_pct))
            .unwrap_or(false);

        let (
            tracked_proc_name,
            tracked_proc_cpu_pct,
            tracked_proc_big_cpu_pct,
            tracked_proc_little_cpu_pct,
            tracked_proc_big_threads,
            tracked_proc_little_threads,
            tracked_proc_rss_mb,
            tracked_proc_eco_qos,
        ) = if let Some(ref procs) = snapshot.tracked_processes {
            if !procs.is_empty() {
                let name = if procs.len() == 1 {
                    Some(procs[0].name.clone())
                } else {
                    procs
                        .iter()
                        .max_by(|a, b| {
                            a.cpu_pct
                                .partial_cmp(&b.cpu_pct)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.cpu_working_set_mb.cmp(&b.cpu_working_set_mb))
                        })
                        .map(|p| p.name.clone())
                };
                let cpu_sum: f32 = procs.iter().map(|p| p.cpu_pct).sum();
                let big_cpu_sum: f32 = procs.iter().map(|p| p.lion_cove_p_cpu_pct).sum();
                let little_cpu_sum: f32 = procs.iter().map(|p| p.skymont_lpe_cpu_pct).sum();
                let big_th_sum: usize = procs.iter().map(|p| p.lion_cove_p_threads).sum();
                let little_th_sum: usize = procs.iter().map(|p| p.skymont_lpe_threads).sum();
                let rss_sum: u64 = procs.iter().map(|p| p.cpu_working_set_mb).sum();
                let eco_qos = procs.iter().any(|p| p.is_eco_qos);

                (
                    name,
                    Some(cpu_sum),
                    Some(big_cpu_sum),
                    Some(little_cpu_sum),
                    Some(big_th_sum as u32),
                    Some(little_th_sum as u32),
                    Some(rss_sum),
                    Some(eco_qos),
                )
            } else {
                (None, None, None, None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None, None, None, None)
        };

        SummarySample {
            interval_secs,
            power_w,
            fan_rpm,
            cpu_total_pct,
            cpu_big_pct,
            cpu_little_pct,
            gpu_duty_pct,
            ram_available_gb,
            disk_read_mb_s,
            disk_write_mb_s,
            disk_util_pct,
            net_rx_mb_s,
            net_tx_mb_s,
            temps,
            accelerator_active,
            accelerator_phase,
            is_heuristic: false,
            prompt_tokens_processed,
            tokens_decoded,
            completed_task_tokens,
            prompt_tps,
            decode_tps,
            tracked_proc_busy,
            marker: snapshot.marker.clone(),
            tracked_proc_name,
            tracked_proc_cpu_pct,
            tracked_proc_big_cpu_pct,
            tracked_proc_little_cpu_pct,
            tracked_proc_big_threads,
            tracked_proc_little_threads,
            tracked_proc_rss_mb,
            tracked_proc_eco_qos,
        }
    }

    fn extract_diagnostics(&self, snapshot: &Self::Snapshot) -> AiDiagnostics {
        let health_status = match snapshot.ai_diagnostics.status {
            DiagnosticStatus::Healthy => "Healthy".to_string(),
            DiagnosticStatus::Degraded => "Degraded".to_string(),
            DiagnosticStatus::Critical => "Critical".to_string(),
        };

        let npu_status = format!(
            "{}: {:.1}% util, {} MHz, status: {:?}",
            snapshot.npu.name,
            snapshot.npu.utilization_pct,
            snapshot.npu.current_freq_mhz,
            snapshot.npu.activity_status
        );

        let peak_temp = snapshot
            .power_thermals
            .cpu_package_dts_c
            .or(snapshot.power_thermals.acpi_thermal_zone_c)
            .or(snapshot.power_thermals.gpu_c)
            .or(snapshot.power_thermals.nvme_composite_c)
            .unwrap_or(45.0);
        let thermal_headroom_c = Some(((100.0 - peak_temp).max(0.0)) as i32);

        let power_efficiency = Some(format!(
            "{:.1} W package ({:?})",
            snapshot.power_thermals.package_w, snapshot.power_thermals.effective_power_mode
        ));

        let summary = format!(
            "LattePanda Mu Ultra: {} status, {} bottlenecks detected",
            health_status,
            snapshot.ai_diagnostics.bottlenecks.len()
        );

        AiDiagnostics {
            health_status,
            active_bottlenecks: snapshot.ai_diagnostics.bottlenecks.clone(),
            npu_status,
            thermal_headroom_c,
            power_efficiency,
            summary,
            recommendations: snapshot.ai_diagnostics.recommendations.clone(),
        }
    }

    fn schema_json(&self) -> &'static str {
        get_schema_json()
    }

    fn schema_fields(&self) -> &'static [SchemaField] {
        SCHEMA_FIELDS
    }

    fn format_oneshot(&self, snapshot: &Self::Snapshot) -> String {
        let mut out = String::new();
        use std::fmt::Write;
        let _ = writeln!(
            out,
            "================================================================================"
        );
        let _ = writeln!(
            out,
            "  MUTOP-AI TELEMETRY SNAPSHOT - {} ({})",
            snapshot.host.board, snapshot.host.soc
        );
        let _ = writeln!(
            out,
            "================================================================================"
        );
        let _ = writeln!(
            out,
            "Privilege: {:?} | TOPS: {} (NPU: {}, GPU: {}, CPU: {}) | Uptime: {:.0}s",
            snapshot.host.privilege_tier,
            snapshot.host.total_ai_tops,
            snapshot.host.npu_tops,
            snapshot.host.gpu_tops,
            snapshot.host.cpu_tops,
            snapshot.timing.unbiased_uptime_sec
        );
        let _ = writeln!(
            out,
            "Timer Resolution: {:.3} ms (min: {:.3} ms, max: {:.3} ms) | Power Mode: {:?}",
            snapshot.timing.timer_resolution.current_ms,
            snapshot.timing.timer_resolution.minimum_ms,
            snapshot.timing.timer_resolution.maximum_ms,
            snapshot.power_thermals.effective_power_mode
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "--- CPU CLUSTERS (8 Cores: 4 Lion Cove P + 4 Skymont LP-E) ---"
        );
        let _ = writeln!(
            out,
            "  Total Load: {:.1}% | Lion Cove P (0..3): {:.1}% | Skymont LP-E (4..7): {:.1}%",
            snapshot.cpu.total_load_pct,
            snapshot.cpu.lion_cove_p_load_pct,
            snapshot.cpu.skymont_lpe_load_pct
        );
        let _ = writeln!(
            out,
            "  P-cores: P0: {:.1}%, P1: {:.1}%, P2: {:.1}%, P3: {:.1}%",
            snapshot.cpu.per_core_pct.first().copied().unwrap_or(0.0),
            snapshot.cpu.per_core_pct.get(1).copied().unwrap_or(0.0),
            snapshot.cpu.per_core_pct.get(2).copied().unwrap_or(0.0),
            snapshot.cpu.per_core_pct.get(3).copied().unwrap_or(0.0)
        );
        let _ = writeln!(
            out,
            "  LP-E cores: E4: {:.1}%, E5: {:.1}%, E6: {:.1}%, E7: {:.1}%",
            snapshot.cpu.per_core_pct.get(4).copied().unwrap_or(0.0),
            snapshot.cpu.per_core_pct.get(5).copied().unwrap_or(0.0),
            snapshot.cpu.per_core_pct.get(6).copied().unwrap_or(0.0),
            snapshot.cpu.per_core_pct.get(7).copied().unwrap_or(0.0)
        );
        let _ = writeln!(
            out,
            "  Conserved: User: {:.1}% | System: {:.1}% | Idle: {:.1}% | Throttled: {}",
            snapshot.cpu.user_pct,
            snapshot.cpu.system_pct,
            snapshot.cpu.idle_pct,
            snapshot.cpu.is_throttled
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "--- ACCELERATORS: {} & {} ---",
            snapshot.gpu.name, snapshot.npu.name
        );
        let _ = writeln!(
            out,
            "  Arc 140V GPU (64 TOPS): {:.1}% (3D: {:.1}%, Compute: {:.1}%, Copy: {:.1}%, Decode: {:.1}%) | Clock: {} MHz | Shared MB: {} / {}",
            snapshot.gpu.total_utilization_pct,
            snapshot.gpu.render_3d_pct,
            snapshot.gpu.compute_pct,
            snapshot.gpu.copy_pct,
            snapshot.gpu.video_decode_pct,
            snapshot.gpu.clock_mhz,
            snapshot.gpu.shared_memory_used_mb,
            snapshot.gpu.shared_memory_budget_mb
        );
        let _ = writeln!(
            out,
            "  AI Boost NPU 4 (47 TOPS): {:.1}% | Freq: {} MHz | Activity: {:?} | Shared MB: {} / {}",
            snapshot.npu.utilization_pct,
            snapshot.npu.current_freq_mhz,
            snapshot.npu.activity_status,
            snapshot.npu.shared_memory_used_mb,
            snapshot.npu.shared_memory_budget_mb
        );
        if let Some(ref slots) = snapshot.npu.slots_metrics {
            let _ = writeln!(
                out,
                "  llama-server: Prefill: {:.1} T/s | Decode: {:.1} T/s | Active Slots: {} / {}",
                slots.prompt_tokens_per_sec,
                slots.generation_tokens_per_sec,
                slots.active_slots,
                slots.total_slots
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "--- UNIFIED MEMORY (16 GB LPDDR5X-8533 MoP) & POWER ---"
        );
        let _ = writeln!(
            out,
            "  In-Use: {} / {} MB ({:.1}%) | Pure CPU: {} MB | Arc Shared: {} MB | NPU Shared: {} MB | Stolen: {} MB",
            snapshot.memory.in_use_phys_mb,
            snapshot.memory.installed_mop_mb,
            (snapshot.memory.in_use_phys_mb as f64 / snapshot.memory.installed_mop_mb as f64) * 100.0,
            snapshot.memory.pure_cpu_in_use_mb,
            snapshot.memory.arc_gpu_shared_mb,
            snapshot.memory.npu_shared_mb,
            snapshot.memory.bios_stolen_aperture_mb
        );
        let _ = writeln!(
            out,
            "  Power: Package (PKG): {:.2} W (Above idle: {:.2} W, Baseline: {:.1} W) | PP0: {:.2} W | PP1: {:.2} W | DRAM: {:.2} W",
            snapshot.power_thermals.package_w,
            snapshot.power_thermals.power_above_idle_w,
            snapshot.power_thermals.idle_baseline_w,
            snapshot.power_thermals.core_pp0_w,
            snapshot.power_thermals.gpu_pp1_w,
            snapshot.power_thermals.dram_w
        );
        let cpu_temp_str = snapshot
            .power_thermals
            .cpu_package_dts_c
            .or(snapshot.power_thermals.acpi_thermal_zone_c)
            .map(|t| format!("{:.1}C", t))
            .unwrap_or_else(|| "-".to_string());
        let gpu_temp_str = snapshot
            .power_thermals
            .gpu_c
            .map(|t| format!("{:.1}C", t))
            .unwrap_or_else(|| "-".to_string());
        let nvme_temp_str = snapshot
            .power_thermals
            .nvme_composite_c
            .map(|t| format!("{:.1}C", t))
            .unwrap_or_else(|| "-".to_string());
        let fan_str = snapshot
            .power_thermals
            .cpu_fan_rpm
            .map(|r| format!("{} RPM", r))
            .unwrap_or_else(|| "Auto".to_string());

        let _ = writeln!(
            out,
            "  Thermals: CPU: {} | GPU: {} | NVMe: {} | Fan: {}",
            cpu_temp_str, gpu_temp_str, nvme_temp_str, fan_str
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "--- STORAGE (NVMe PCIe Gen{} x{}) & NETWORK ---",
            snapshot.storage.current_gen, snapshot.storage.current_lanes
        );
        let _ = writeln!(
            out,
            "  NVMe: {:.1} / {:.1} GB ({:.1}%) | Read: {:.2} MB/s | Write: {:.2} MB/s | Busy: {:.1}% | Degraded: {}",
            snapshot.storage.capacity_gb - snapshot.storage.free_gb,
            snapshot.storage.capacity_gb,
            snapshot.storage.used_pct,
            snapshot.storage.read_mb_s,
            snapshot.storage.write_mb_s,
            snapshot.storage.busy_pct,
            snapshot.storage.is_link_degraded
        );

        let total_rx_mbps: f64 = snapshot.network.interfaces.iter().map(|i| i.rx_mbps).sum();
        let total_tx_mbps: f64 = snapshot.network.interfaces.iter().map(|i| i.tx_mbps).sum();
        let _ = writeln!(
            out,
            "  Network: Rx: {:.2} Mbps | Tx: {:.2} Mbps | Loopback llama-server:8080: {} | MCP Gateway: {}",
            total_rx_mbps,
            total_tx_mbps,
            if snapshot.network.loopback.llama_server_listening {
                "Listening"
            } else {
                "Offline"
            },
            if snapshot.network.loopback.mcp_gateway_listening {
                "Listening"
            } else {
                "Offline"
            }
        );
        if let Some(ref wifi) = snapshot.network.wifi7 {
            let _ = writeln!(
                out,
                "  Wi-Fi 7: RSSI: {} dBm | Channel: {} | Gen: {} (MLO: {})",
                wifi.rssi_dbm, wifi.channel, wifi.phy_type, wifi.is_mlo
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "--- AI DIAGNOSTICS: {:?} ---",
            snapshot.ai_diagnostics.status
        );
        if !snapshot.ai_diagnostics.bottlenecks.is_empty() {
            let _ = writeln!(
                out,
                "  Bottlenecks: {}",
                snapshot.ai_diagnostics.bottlenecks.join(", ")
            );
        }
        if !snapshot.ai_diagnostics.recommendations.is_empty() {
            let _ = writeln!(
                out,
                "  Recommendations: {}",
                snapshot.ai_diagnostics.recommendations.join(" | ")
            );
        }

        if let Some(ref procs) = snapshot.tracked_processes {
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "--- TRACKED PROCESSES & PLACEMENT ({}) ---",
                procs.len()
            );
            for p in procs {
                let _ = writeln!(
                    out,
                    "  [{}] {:<20} CPU: {:>5.1}% (P: {:>5.1}%, LP-E: {:>5.1}%) | Mem: {:>4} MB | GPU: {:>4} MB | NPU: {:>4} MB | Threads: P:{}/LP-E:{} | EcoQoS: {}",
                    p.pid,
                    p.name,
                    p.cpu_pct,
                    p.lion_cove_p_cpu_pct,
                    p.skymont_lpe_cpu_pct,
                    p.cpu_working_set_mb,
                    p.gpu_shared_mb,
                    p.npu_shared_mb,
                    p.lion_cove_p_threads,
                    p.skymont_lpe_threads,
                    p.is_eco_qos
                );
            }
        }
        out
    }

    fn current_tracked_processes(&self, snapshot: &Self::Snapshot) -> Vec<(u32, String)> {
        snapshot
            .tracked_processes
            .as_ref()
            .map(|procs| procs.iter().map(|p| (p.pid, p.name.clone())).collect())
            .unwrap_or_default()
    }
}
