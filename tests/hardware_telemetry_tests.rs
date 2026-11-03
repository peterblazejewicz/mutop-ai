// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use mutop_ai::hardware::diagnostics::evaluate_diagnostics;
use mutop_ai::hardware::memory::collect_memory_snapshot;
use mutop_ai::hardware::{CollectorMode, HardwareCollector};
use mutop_ai::snapshot::types::*;
use mutop_ai::snapshot::{get_schema_json, MutopSnapshot};
use std::collections::HashMap;

#[test]
fn test_mock_hardware_collector() {
    let mut collector = HardwareCollector::new_mock();
    assert_eq!(collector.mode, CollectorMode::Mock);

    let snapshot = collector.collect_snapshot(
        Some("benchmark_phase_1".to_string()),
        &["llama-server".to_string()],
    );

    assert_eq!(snapshot.marker.as_deref(), Some("benchmark_phase_1"));
    assert_eq!(
        snapshot.host.board,
        "LattePanda Mu Ultra (DFR1295 + DFR1293)"
    );
    assert_eq!(snapshot.host.soc, "Intel Core Ultra 7 256V (Lunar Lake)");
    assert_eq!(snapshot.host.total_ai_tops, 115);
    assert_eq!(snapshot.host.npu_tops, 47);
    assert_eq!(snapshot.host.gpu_tops, 64);
    assert_eq!(snapshot.host.cpu_tops, 5);

    // CPU checks
    assert_eq!(snapshot.cpu.per_core_pct.len(), 8);
    assert_eq!(snapshot.cpu.per_core_microarch.len(), 8);
    for i in 0..4 {
        assert_eq!(snapshot.cpu.per_core_microarch[i], CoreMicroarch::LionCoveP);
    }
    for i in 4..8 {
        assert_eq!(
            snapshot.cpu.per_core_microarch[i],
            CoreMicroarch::SkymontLpE
        );
    }
    assert!(!snapshot.cpu.is_throttled);

    // Memory non-double-counting check
    assert_eq!(snapshot.memory.installed_mop_mb, 16384);
    assert_eq!(
        snapshot.memory.bios_stolen_aperture_mb,
        snapshot.memory.installed_mop_mb - snapshot.memory.os_total_phys_mb
    );
    assert_eq!(
        snapshot.memory.in_use_phys_mb,
        snapshot.memory.os_total_phys_mb - snapshot.memory.os_avail_phys_mb
    );
    assert_eq!(
        snapshot.memory.pure_cpu_in_use_mb,
        snapshot
            .memory
            .in_use_phys_mb
            .saturating_sub(snapshot.memory.arc_gpu_shared_mb + snapshot.memory.npu_shared_mb)
    );

    // GPU checks
    assert_eq!(snapshot.gpu.name, "Intel(R) Arc(TM) 140V GPU");
    assert_eq!(snapshot.gpu.gpu_tops, 64);

    // NPU checks
    assert_eq!(snapshot.npu.name, "Intel(R) AI Boost");
    assert_eq!(snapshot.npu.npu_tops, 47);

    // Storage checks
    assert_eq!(snapshot.storage.current_gen, 4);
    assert_eq!(snapshot.storage.current_lanes, 4);
    assert!(!snapshot.storage.is_link_degraded);

    // Network checks
    assert_eq!(snapshot.network.interfaces.len(), 2);
    assert!(snapshot.network.wifi7.is_some());
    let wifi = snapshot.network.wifi7.as_ref().unwrap();
    assert_eq!(wifi.phy_type, "EHT (802.11be)");
    assert!(wifi.is_mlo);
    assert!(snapshot.network.loopback.llama_server_listening);

    // Timing checks
    assert!(snapshot.timing.tsc_invariant);
    assert_eq!(snapshot.timing.crystal_hz, 38_400_000);
    assert!(snapshot.timing.timer_resolution.is_high_resolution_1ms);

    // Diagnostics checks
    assert_eq!(snapshot.ai_diagnostics.status, DiagnosticStatus::Healthy);
    assert!(!snapshot.ai_diagnostics.llm_worker_on_skymont_lpe_cores);
}

#[test]
fn test_snapshot_serialization_and_deserialization() {
    let mut collector = HardwareCollector::new_mock();
    let snapshot = collector.collect_snapshot(None, &[]);

    let json_str = serde_json::to_string_pretty(&snapshot).expect("serialize snapshot");
    assert!(!json_str.is_empty());

    let deserialized: MutopSnapshot =
        serde_json::from_str(&json_str).expect("deserialize snapshot");
    assert_eq!(deserialized.host.board, snapshot.host.board);
    assert_eq!(deserialized.cpu.total_load_pct, snapshot.cpu.total_load_pct);
    assert_eq!(deserialized.memory.installed_mop_mb, 16384);
    assert_eq!(deserialized.npu.npu_tops, 47);
    assert_eq!(deserialized.gpu.gpu_tops, 64);
}

#[test]
fn test_schema_json_validity() {
    let schema_str = get_schema_json();
    let schema_val: serde_json::Value =
        serde_json::from_str(schema_str).expect("schema must be valid JSON");

    assert_eq!(schema_val["title"], "MutopSnapshot");
    let properties = schema_val["properties"]
        .as_object()
        .expect("properties object");

    let expected_keys = [
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
        "ai_diagnostics",
    ];

    for key in expected_keys {
        assert!(
            properties.contains_key(key),
            "Schema missing property '{}'",
            key
        );
    }
}

#[test]
fn test_uma_memory_model_math() {
    let mem = collect_memory_snapshot(2048, 256, true);
    assert_eq!(mem.installed_mop_mb, 16384);
    assert_eq!(mem.arc_gpu_shared_mb, 2048);
    assert_eq!(mem.npu_shared_mb, 256);
    assert_eq!(
        mem.pure_cpu_in_use_mb,
        mem.in_use_phys_mb.saturating_sub(2048 + 256)
    );
}

#[test]
fn test_ai_diagnostics_rules() {
    let mut collector = HardwareCollector::new_mock();
    let snapshot = collector.collect_snapshot(None, &[]);

    // Rule 1: LLM worker on LP-E cores
    let bad_process = ProcessAffinityProfile {
        pid: 9999,
        name: "llama-server.exe".to_string(),
        cmdline: "llama-server".to_string(),
        cpu_pct: 30.0,
        lion_cove_p_cpu_pct: 10.0,
        skymont_lpe_cpu_pct: 20.0,
        cpu_working_set_mb: 512,
        gpu_shared_mb: 0,
        npu_shared_mb: 0,
        commit_private_mb: 512,
        thread_count: 4,
        lion_cove_p_threads: 1,
        skymont_lpe_threads: 3, // trapped on LP-E cores!
        is_eco_qos: true,
        core_distribution: HashMap::new(),
        threads: vec![],
    };

    let diag = evaluate_diagnostics(
        &snapshot.cpu,
        &snapshot.memory,
        &snapshot.gpu,
        &snapshot.power_thermals,
        &snapshot.storage,
        &snapshot.timing,
        Some(std::slice::from_ref(&bad_process)),
        false,
    );
    assert!(diag.llm_worker_on_skymont_lpe_cores);
    assert_eq!(diag.status, DiagnosticStatus::Degraded);
    assert!(diag.bottlenecks.iter().any(|b| b.contains("Skymont LP-E")));

    // Rule 2: Coarse timer resolution during active voice turn
    let mut coarse_timing = snapshot.timing.clone();
    coarse_timing.timer_resolution.current_ms = 15.625;
    let diag_timer = evaluate_diagnostics(
        &snapshot.cpu,
        &snapshot.memory,
        &snapshot.gpu,
        &snapshot.power_thermals,
        &snapshot.storage,
        &coarse_timing,
        None,
        true, // marker active
    );
    assert!(diag_timer.timer_resolution_coarse_during_voice_turn);
    assert_eq!(diag_timer.status, DiagnosticStatus::Degraded);

    // Rule 5: Degraded PCIe link
    let mut degraded_storage = snapshot.storage.clone();
    degraded_storage.is_link_degraded = true;
    degraded_storage.current_gen = 3;
    let diag_pcie = evaluate_diagnostics(
        &snapshot.cpu,
        &snapshot.memory,
        &snapshot.gpu,
        &snapshot.power_thermals,
        &degraded_storage,
        &snapshot.timing,
        None,
        false,
    );
    assert!(diag_pcie.nvme_pcie_link_downgraded);

    // Rule 6: Critical SMART thermal throttling
    let mut throttling_storage = snapshot.storage.clone();
    if let Some(ref mut smart) = throttling_storage.smart_health {
        smart.warning_throttle_minutes = 5;
    }
    let diag_crit = evaluate_diagnostics(
        &snapshot.cpu,
        &snapshot.memory,
        &snapshot.gpu,
        &snapshot.power_thermals,
        &throttling_storage,
        &snapshot.timing,
        None,
        false,
    );
    assert!(diag_crit.nvme_thermal_throttling);
    assert_eq!(diag_crit.status, DiagnosticStatus::Critical);
}

#[test]
fn test_individual_collectors() {
    use mutop_ai::hardware::*;

    let host = get_host_info(true);
    assert_eq!(host.total_ai_tops, 115);

    let mut cpu_state = CpuCollectorState::default();
    let cpu = collect_cpu_snapshot(&mut cpu_state, true);
    assert_eq!(cpu.per_core_pct.len(), 8);

    let npu = collect_npu_snapshot(None, true);
    assert_eq!(npu.npu_tops, 47);

    let gpu = collect_gpu_snapshot(None, true);
    assert_eq!(gpu.gpu_tops, 64);

    let power = collect_power_thermal_snapshot(None, 4.8, true);
    assert!(power.package_w > 0.0);

    let storage = collect_storage_snapshot(None, true);
    assert_eq!(storage.current_gen, 4);

    let mut net_state = NetworkCollectorState::default();
    let network = collect_network_snapshot(&mut net_state, true);
    assert!(!network.interfaces.is_empty());

    let timing = collect_timing_snapshot(true);
    assert_eq!(timing.crystal_hz, 38_400_000);

    let procs = collect_tracked_processes(None, &["llama".to_string()], true);
    assert!(!procs.is_empty());
}

#[test]
fn test_print_schema_and_version() {
    use mutop_ai::{print_schema, version};
    assert_eq!(version(), "0.1.0");
    // Verify print_schema runs cleanly
    print_schema();
}

#[test]
#[ignore = "Live hardware verification runs only on LattePanda Mu Ultra (MU) target"]
fn test_live_collector_graceful_run() {
    let mut collector = HardwareCollector::new();
    let snapshot = collector.collect_snapshot(None, &[]);
    assert_eq!(snapshot.host.total_ai_tops, 115);
    assert_eq!(snapshot.memory.installed_mop_mb, 16384);
}
