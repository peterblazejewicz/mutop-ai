// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivilegeTier {
    Tier1Unprivileged,
    Tier2ElevatedAdmin,
    Tier3SignedDriver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub board: String,
    pub soc: String,
    pub hostname: String,
    pub os_build: String,
    pub architecture: String,
    pub privilege_tier: PrivilegeTier,
    pub total_ai_tops: u32,
    pub npu_tops: u32,
    pub gpu_tops: u32,
    pub cpu_tops: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoreMicroarch {
    LionCoveP,
    SkymontLpE,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunarLakeCpuSnapshot {
    pub total_load_pct: f32,
    pub lion_cove_p_load_pct: f32,
    pub skymont_lpe_load_pct: f32,
    pub per_core_pct: Vec<f32>,
    pub per_core_effective_mhz: Vec<u32>,
    pub per_core_current_mhz: Vec<u32>,
    pub per_core_max_mhz: Vec<u32>,
    pub per_core_limit_mhz: Vec<u32>,
    pub per_core_microarch: Vec<CoreMicroarch>,
    pub is_throttled: bool,
    pub user_pct: f64,
    pub system_pct: f64,
    pub idle_pct: f64,
    pub iowait_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dpc_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_pct: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NpuActivityStatus {
    Idle,
    ActivePromptProcessing,
    ActiveTokenGeneration,
    ActiveCompute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaSlotsMetrics {
    pub prompt_tokens_per_sec: f64,
    pub generation_tokens_per_sec: f64,
    pub active_slots: usize,
    pub total_slots: usize,
    #[serde(default)]
    pub prompt_tokens_processed: u32,
    #[serde(default)]
    pub tokens_decoded: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_task_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceleratorProcess {
    pub pid: u32,
    pub name: String,
    pub engine: String,
    pub shared_memory_mb: u64,
    pub utilization_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelNpuSnapshot {
    pub name: String,
    pub model: String,
    pub npu_tops: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_luid: Option<u64>,
    pub utilization_pct: f32,
    pub current_freq_mhz: u32,
    pub shared_memory_used_mb: u64,
    pub shared_memory_budget_mb: u64,
    pub activity_status: NpuActivityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slots_metrics: Option<LlamaSlotsMetrics>,
    pub active_processes: Vec<AcceleratorProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuThrottlingFlags {
    pub power_limited: bool,
    pub temp_limited: bool,
    pub current_limited: bool,
    pub voltage_limited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelArcGpuSnapshot {
    pub name: String,
    pub architecture: String,
    pub gpu_tops: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_luid: Option<u64>,
    pub total_utilization_pct: f32,
    pub render_3d_pct: f32,
    pub compute_pct: f32,
    pub copy_pct: f32,
    pub video_decode_pct: f32,
    pub clock_mhz: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_w: Option<f32>,
    pub throttling: GpuThrottlingFlags,
    pub shared_memory_used_mb: u64,
    pub shared_memory_budget_mb: u64,
    pub active_processes: Vec<AcceleratorProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UmaMemorySnapshot {
    pub installed_mop_mb: u64,
    pub bios_stolen_aperture_mb: u64,
    pub os_total_phys_mb: u64,
    pub os_avail_phys_mb: u64,
    pub in_use_phys_mb: u64,
    pub pure_cpu_in_use_mb: u64,
    pub arc_gpu_shared_mb: u64,
    pub npu_shared_mb: u64,
    pub commit_total_mb: u64,
    pub commit_limit_mb: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standby_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_zero_mb: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EffectivePowerMode {
    BatterySaver,
    BestPowerEfficiency,
    #[default]
    Balanced,
    BestPerformance,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerThermalSnapshot {
    pub package_w: f64,
    pub core_pp0_w: f64,
    pub gpu_pp1_w: f64,
    pub dram_w: f64,
    pub power_above_idle_w: f64,
    pub idle_baseline_w: f64,
    pub ac_online: bool,
    pub battery_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discharge_rate_w: Option<f64>,
    pub effective_power_mode: EffectivePowerMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acpi_thermal_zone_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvme_composite_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvme_asic_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvme_nand_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_package_dts_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_core_dts_c: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_fan_rpm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NvmeSmartHealth {
    pub composite_temp_c: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nand_temp_c: Option<f32>,
    pub available_spare_pct: u8,
    pub percentage_used_wear: u8,
    pub data_read_gb: u64,
    pub data_written_gb: u64,
    pub unsafe_shutdowns: u64,
    pub warning_throttle_minutes: u32,
    pub critical_throttle_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NvmeStorageSnapshot {
    pub capacity_gb: f64,
    pub free_gb: f64,
    pub used_pct: f32,
    pub current_gen: u8,
    pub current_lanes: u8,
    pub max_gen: u8,
    pub max_lanes: u8,
    pub is_link_degraded: bool,
    pub read_mb_s: f64,
    pub write_mb_s: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    pub queue_depth: f64,
    pub avg_read_latency_ms: f64,
    pub avg_write_latency_ms: f64,
    pub busy_pct: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smart_health: Option<NvmeSmartHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceSnapshot {
    pub name: String,
    pub rx_mbps: f64,
    pub tx_mbps: f64,
    pub link_speed_mbps: u64,
    pub oper_status: String,
    pub in_errors: u64,
    pub out_discards: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wifi7Snapshot {
    pub phy_type: String,
    pub rssi_dbm: i32,
    pub signal_quality_pct: u8,
    pub rx_rate_mbps: u32,
    pub tx_rate_mbps: u32,
    pub channel: u32,
    pub is_mlo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopbackAiSnapshot {
    pub llama_server_listening: bool,
    pub llama_server_port: u16,
    pub mcp_gateway_listening: bool,
    pub mcp_gateway_port: u16,
    pub active_loopback_connections: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loopback_bytes_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loopback_bytes_out: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkLoopbackSnapshot {
    pub interfaces: Vec<NetworkInterfaceSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wifi7: Option<Wifi7Snapshot>,
    pub loopback: LoopbackAiSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerResolutionSnapshot {
    pub current_ms: f64,
    pub minimum_ms: f64,
    pub maximum_ms: f64,
    pub is_high_resolution_1ms: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemTimingSnapshot {
    pub tsc_invariant: bool,
    pub crystal_hz: u64,
    pub tsc_hz: u64,
    pub qpc_frequency_hz: u64,
    pub unbiased_uptime_sec: f64,
    pub timer_resolution: TimerResolutionSnapshot,
    pub adjustment_disabled: bool,
    pub frequency_slew_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtc_battery_low: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunarLakeThreadPlacement {
    pub tid: u32,
    pub name: String,
    pub cpu_core: u32,
    pub microarch: CoreMicroarch,
    pub is_eco_qos: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessAffinityProfile {
    pub pid: u32,
    pub name: String,
    pub cmdline: String,
    pub cpu_pct: f32,
    pub lion_cove_p_cpu_pct: f32,
    pub skymont_lpe_cpu_pct: f32,
    pub cpu_working_set_mb: u64,
    pub gpu_shared_mb: u64,
    pub npu_shared_mb: u64,
    pub commit_private_mb: u64,
    pub thread_count: usize,
    pub lion_cove_p_threads: usize,
    pub skymont_lpe_threads: usize,
    pub is_eco_qos: bool,
    pub core_distribution: HashMap<String, usize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<LunarLakeThreadPlacement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticStatus {
    Healthy,
    Degraded,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiDiagnostics {
    pub status: DiagnosticStatus,
    pub llm_worker_on_skymont_lpe_cores: bool,
    pub timer_resolution_coarse_during_voice_turn: bool,
    pub uma_memory_pressure_near_budget: bool,
    pub gpu_or_cpu_power_throttling: bool,
    pub nvme_pcie_link_downgraded: bool,
    pub nvme_thermal_throttling: bool,
    pub bottlenecks: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutopSnapshot {
    pub timestamp_unix: u64,
    pub timestamp_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    pub host: HostInfo,
    pub cpu: LunarLakeCpuSnapshot,
    pub memory: UmaMemorySnapshot,
    pub gpu: IntelArcGpuSnapshot,
    pub npu: IntelNpuSnapshot,
    pub power_thermals: PowerThermalSnapshot,
    pub storage: NvmeStorageSnapshot,
    pub network: NetworkLoopbackSnapshot,
    pub timing: SystemTimingSnapshot,
    pub ai_diagnostics: AiDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracked_processes: Option<Vec<ProcessAffinityProfile>>,
    #[serde(default)]
    pub events: Vec<edgetop_core::ProcessLifecycleEvent>,
}
