// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

pub fn get_schema_json() -> &'static str {
    r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "MutopSnapshot",
  "type": "object",
  "properties": {
    "timestamp_unix": { "type": "integer", "description": "Unix epoch timestamp in seconds" },
    "timestamp_unix_ms": { "type": "integer", "description": "Unix epoch timestamp in milliseconds" },
    "interval_ms": { "type": ["number", "null"], "description": "Stream tick interval in milliseconds" },
    "marker": { "type": ["string", "null"], "description": "Phase marker string" },
    "host": {
      "type": "object",
      "properties": {
        "board": { "type": "string" },
        "soc": { "type": "string" },
        "hostname": { "type": "string" },
        "os_build": { "type": "string" },
        "architecture": { "type": "string" },
        "privilege_tier": { "type": "string", "enum": ["Tier1Unprivileged", "Tier2ElevatedAdmin", "Tier3SignedDriver"] },
        "total_ai_tops": { "type": "integer" },
        "npu_tops": { "type": "integer" },
        "gpu_tops": { "type": "integer" },
        "cpu_tops": { "type": "integer" }
      },
      "required": ["board", "soc", "hostname", "os_build", "architecture", "privilege_tier", "total_ai_tops", "npu_tops", "gpu_tops", "cpu_tops"]
    },
    "cpu": {
      "type": "object",
      "properties": {
        "total_load_pct": { "type": "number" },
        "lion_cove_p_load_pct": { "type": "number" },
        "skymont_lpe_load_pct": { "type": "number" },
        "per_core_pct": { "type": "array", "items": { "type": "number" } },
        "per_core_effective_mhz": { "type": "array", "items": { "type": "integer" } },
        "per_core_current_mhz": { "type": "array", "items": { "type": "integer" } },
        "per_core_max_mhz": { "type": "array", "items": { "type": "integer" } },
        "per_core_limit_mhz": { "type": "array", "items": { "type": "integer" } },
        "per_core_microarch": { "type": "array", "items": { "type": "string", "enum": ["LionCoveP", "SkymontLpE"] } },
        "is_throttled": { "type": "boolean" },
        "user_pct": { "type": "number" },
        "system_pct": { "type": "number" },
        "idle_pct": { "type": "number" },
        "iowait_pct": { "type": "number" },
        "dpc_pct": { "type": ["number", "null"] },
        "interrupt_pct": { "type": ["number", "null"] }
      },
      "required": ["total_load_pct", "lion_cove_p_load_pct", "skymont_lpe_load_pct", "per_core_pct", "per_core_effective_mhz", "per_core_current_mhz", "per_core_max_mhz", "per_core_limit_mhz", "per_core_microarch", "is_throttled", "user_pct", "system_pct", "idle_pct", "iowait_pct"]
    },
    "memory": {
      "type": "object",
      "properties": {
        "installed_mop_mb": { "type": "integer" },
        "bios_stolen_aperture_mb": { "type": "integer" },
        "os_total_phys_mb": { "type": "integer" },
        "os_avail_phys_mb": { "type": "integer" },
        "in_use_phys_mb": { "type": "integer" },
        "pure_cpu_in_use_mb": { "type": "integer" },
        "arc_gpu_shared_mb": { "type": "integer" },
        "npu_shared_mb": { "type": "integer" },
        "commit_total_mb": { "type": "integer" },
        "commit_limit_mb": { "type": "integer" },
        "standby_mb": { "type": ["integer", "null"] },
        "modified_mb": { "type": ["integer", "null"] },
        "free_zero_mb": { "type": ["integer", "null"] }
      },
      "required": ["installed_mop_mb", "bios_stolen_aperture_mb", "os_total_phys_mb", "os_avail_phys_mb", "in_use_phys_mb", "pure_cpu_in_use_mb", "arc_gpu_shared_mb", "npu_shared_mb", "commit_total_mb", "commit_limit_mb"]
    },
    "gpu": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "architecture": { "type": "string" },
        "gpu_tops": { "type": "integer" },
        "adapter_luid": { "type": ["integer", "null"] },
        "total_utilization_pct": { "type": "number" },
        "render_3d_pct": { "type": "number" },
        "compute_pct": { "type": "number" },
        "copy_pct": { "type": "number" },
        "video_decode_pct": { "type": "number" },
        "clock_mhz": { "type": "integer" },
        "temperature_c": { "type": ["number", "null"] },
        "power_w": { "type": ["number", "null"] },
        "throttling": {
          "type": "object",
          "properties": {
            "power_limited": { "type": "boolean" },
            "temp_limited": { "type": "boolean" },
            "current_limited": { "type": "boolean" },
            "voltage_limited": { "type": "boolean" }
          },
          "required": ["power_limited", "temp_limited", "current_limited", "voltage_limited"]
        },
        "shared_memory_used_mb": { "type": "integer" },
        "shared_memory_budget_mb": { "type": "integer" },
        "active_processes": { "type": "array" }
      },
      "required": ["name", "architecture", "gpu_tops", "total_utilization_pct", "render_3d_pct", "compute_pct", "copy_pct", "video_decode_pct", "clock_mhz", "throttling", "shared_memory_used_mb", "shared_memory_budget_mb", "active_processes"]
    },
    "npu": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "model": { "type": "string" },
        "npu_tops": { "type": "integer" },
        "adapter_luid": { "type": ["integer", "null"] },
        "utilization_pct": { "type": "number" },
        "current_freq_mhz": { "type": "integer" },
        "shared_memory_used_mb": { "type": "integer" },
        "shared_memory_budget_mb": { "type": "integer" },
        "activity_status": { "type": "string", "enum": ["Idle", "ActivePromptProcessing", "ActiveTokenGeneration", "ActiveCompute"] },
        "slots_metrics": { "type": ["object", "null"] },
        "active_processes": { "type": "array" }
      },
      "required": ["name", "model", "npu_tops", "utilization_pct", "current_freq_mhz", "shared_memory_used_mb", "shared_memory_budget_mb", "activity_status", "active_processes"]
    },
    "power_thermals": {
      "type": "object",
      "properties": {
        "package_w": { "type": "number" },
        "core_pp0_w": { "type": "number" },
        "gpu_pp1_w": { "type": "number" },
        "dram_w": { "type": "number" },
        "power_above_idle_w": { "type": "number" },
        "idle_baseline_w": { "type": "number" },
        "ac_online": { "type": "boolean" },
        "battery_present": { "type": "boolean" },
        "battery_pct": { "type": ["integer", "null"] },
        "discharge_rate_w": { "type": ["number", "null"] },
        "effective_power_mode": { "type": "string", "enum": ["BatterySaver", "BestPowerEfficiency", "Balanced", "BestPerformance", "Unknown"] },
        "acpi_thermal_zone_c": { "type": ["number", "null"] },
        "gpu_c": { "type": ["number", "null"] },
        "nvme_composite_c": { "type": ["number", "null"] },
        "nvme_asic_c": { "type": ["number", "null"] },
        "nvme_nand_c": { "type": ["number", "null"] },
        "cpu_package_dts_c": { "type": ["number", "null"] },
        "cpu_core_dts_c": { "type": ["array", "null"] },
        "cpu_fan_rpm": { "type": ["integer", "null"] },
        "thermal_status": { "type": ["string", "null"] }
      },
      "required": ["package_w", "core_pp0_w", "gpu_pp1_w", "dram_w", "power_above_idle_w", "idle_baseline_w", "ac_online", "battery_present", "effective_power_mode"]
    },
    "storage": {
      "type": "object",
      "properties": {
        "capacity_gb": { "type": "number" },
        "free_gb": { "type": "number" },
        "used_pct": { "type": "number" },
        "current_gen": { "type": "integer" },
        "current_lanes": { "type": "integer" },
        "max_gen": { "type": "integer" },
        "max_lanes": { "type": "integer" },
        "is_link_degraded": { "type": "boolean" },
        "read_mb_s": { "type": "number" },
        "write_mb_s": { "type": "number" },
        "read_iops": { "type": "number" },
        "write_iops": { "type": "number" },
        "queue_depth": { "type": "number" },
        "avg_read_latency_ms": { "type": "number" },
        "avg_write_latency_ms": { "type": "number" },
        "busy_pct": { "type": "number" },
        "smart_health": { "type": ["object", "null"] }
      },
      "required": ["capacity_gb", "free_gb", "used_pct", "current_gen", "current_lanes", "max_gen", "max_lanes", "is_link_degraded", "read_mb_s", "write_mb_s", "read_iops", "write_iops", "queue_depth", "avg_read_latency_ms", "avg_write_latency_ms", "busy_pct"]
    },
    "network": {
      "type": "object",
      "properties": {
        "interfaces": { "type": "array" },
        "wifi7": { "type": ["object", "null"] },
        "loopback": { "type": "object" }
      },
      "required": ["interfaces", "loopback"]
    },
    "timing": {
      "type": "object",
      "properties": {
        "tsc_invariant": { "type": "boolean" },
        "crystal_hz": { "type": "integer" },
        "tsc_hz": { "type": "integer" },
        "qpc_frequency_hz": { "type": "integer" },
        "unbiased_uptime_sec": { "type": "number" },
        "timer_resolution": { "type": "object" },
        "adjustment_disabled": { "type": "boolean" },
        "frequency_slew_ratio": { "type": "number" },
        "rtc_battery_low": { "type": ["boolean", "null"] }
      },
      "required": ["tsc_invariant", "crystal_hz", "tsc_hz", "qpc_frequency_hz", "unbiased_uptime_sec", "timer_resolution", "adjustment_disabled", "frequency_slew_ratio"]
    },
    "ai_diagnostics": {
      "type": "object",
      "properties": {
        "status": { "type": "string", "enum": ["Healthy", "Degraded", "Critical"] },
        "llm_worker_on_skymont_lpe_cores": { "type": "boolean" },
        "timer_resolution_coarse_during_voice_turn": { "type": "boolean" },
        "uma_memory_pressure_near_budget": { "type": "boolean" },
        "gpu_or_cpu_power_throttling": { "type": "boolean" },
        "nvme_pcie_link_downgraded": { "type": "boolean" },
        "nvme_thermal_throttling": { "type": "boolean" },
        "bottlenecks": { "type": "array", "items": { "type": "string" } },
        "recommendations": { "type": "array", "items": { "type": "string" } }
      },
      "required": ["status", "llm_worker_on_skymont_lpe_cores", "timer_resolution_coarse_during_voice_turn", "uma_memory_pressure_near_budget", "gpu_or_cpu_power_throttling", "nvme_pcie_link_downgraded", "nvme_thermal_throttling", "bottlenecks", "recommendations"]
    },
    "tracked_processes": { "type": ["array", "null"] },
    "events": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "event": { "type": "string" },
          "pid": { "type": "integer" },
          "name": { "type": "string" },
          "timestamp_unix_ms": { "type": "integer" }
        },
        "required": ["event", "pid", "name", "timestamp_unix_ms"]
      }
    }
  },
  "required": [
    "timestamp_unix",
    "timestamp_unix_ms",
    "host",
    "cpu",
    "memory",
    "gpu",
    "npu",
    "power_thermals",
    "storage",
    "network",
    "timing",
    "ai_diagnostics"
  ]
}"#
}

pub const SCHEMA_FIELDS: &[edgetop_core::SchemaField] = &[
    edgetop_core::SchemaField {
        field: "timestamp_unix",
        field_type: "u64",
        description: "Unix epoch timestamp in seconds",
    },
    edgetop_core::SchemaField {
        field: "timestamp_unix_ms",
        field_type: "u64",
        description: "Unix epoch timestamp in milliseconds",
    },
    edgetop_core::SchemaField {
        field: "interval_ms",
        field_type: "f64?",
        description: "Sample interval in milliseconds",
    },
    edgetop_core::SchemaField {
        field: "marker",
        field_type: "string?",
        description: "External benchmark phase marker",
    },
    edgetop_core::SchemaField {
        field: "events[]",
        field_type: "array",
        description: "Process lifecycle transitions (started/exited PIDs) in tick",
    },
    edgetop_core::SchemaField {
        field: "host.board",
        field_type: "string",
        description: "Carrier board + compute module model",
    },
    edgetop_core::SchemaField {
        field: "host.soc",
        field_type: "string",
        description: "SoC identification (Lunar Lake 256V)",
    },
    edgetop_core::SchemaField {
        field: "host.privilege_tier",
        field_type: "enum",
        description: "Tier1Unprivileged | Tier2ElevatedAdmin | Tier3SignedDriver",
    },
    edgetop_core::SchemaField {
        field: "host.total_ai_tops",
        field_type: "u32",
        description: "Combined platform AI TOPS (115 TOPS)",
    },
    edgetop_core::SchemaField {
        field: "cpu.total_load_pct",
        field_type: "f32",
        description: "Total 8-core CPU utilization %",
    },
    edgetop_core::SchemaField {
        field: "cpu.lion_cove_p_load_pct",
        field_type: "f32",
        description: "Lion Cove P-cores (0..3) load %",
    },
    edgetop_core::SchemaField {
        field: "cpu.skymont_lpe_load_pct",
        field_type: "f32",
        description: "Skymont LP-E cores (4..7) load %",
    },
    edgetop_core::SchemaField {
        field: "cpu.is_throttled",
        field_type: "bool",
        description: "True if current clock limit < max clock",
    },
    edgetop_core::SchemaField {
        field: "memory.installed_mop_mb",
        field_type: "u64",
        description: "Total 16GB LPDDR5X-8533 MoP (16384 MB)",
    },
    edgetop_core::SchemaField {
        field: "memory.in_use_phys_mb",
        field_type: "u64",
        description: "Physical in-use memory (OS Total - Avail)",
    },
    edgetop_core::SchemaField {
        field: "memory.pure_cpu_in_use_mb",
        field_type: "u64",
        description: "Reconciled CPU RAM excluding GPU & NPU shared",
    },
    edgetop_core::SchemaField {
        field: "gpu.total_utilization_pct",
        field_type: "f32",
        description: "Intel Arc 140V total utilization %",
    },
    edgetop_core::SchemaField {
        field: "npu.utilization_pct",
        field_type: "f32",
        description: "Intel NPU 4 (47 TOPS) utilization %",
    },
    edgetop_core::SchemaField {
        field: "power_thermals.package_w",
        field_type: "f64",
        description: "RAPL package power in Watts",
    },
    edgetop_core::SchemaField {
        field: "power_thermals.cpu_package_dts_c",
        field_type: "f32?",
        description: "SoC Compute Tile (CPU/GPU/NPU) package DTS in Celsius",
    },
    edgetop_core::SchemaField {
        field: "power_thermals.thermal_status",
        field_type: "string?",
        description: "Thermal sensor availability or status message",
    },
    edgetop_core::SchemaField {
        field: "storage.read_mb_s",
        field_type: "f64",
        description: "NVMe disk read throughput MB/s",
    },
    edgetop_core::SchemaField {
        field: "timing.timer_resolution",
        field_type: "object",
        description: "Windows system timer resolution (ms)",
    },
    edgetop_core::SchemaField {
        field: "ai_diagnostics.status",
        field_type: "enum",
        description: "Healthy | Degraded | Critical",
    },
];

pub fn print_schema() {
    let meta = edgetop_core::PlatformMetadata {
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
    };
    print!(
        "{}",
        edgetop_core::schema::format_schema_table(&meta, SCHEMA_FIELDS)
    );
}
