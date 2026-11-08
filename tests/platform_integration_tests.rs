// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use edgetop_core::cli::{parse_args, CliAction};
use edgetop_core::mcp::handle_mcp_request;
use edgetop_core::stream::{run_stream, StreamOptions};
use edgetop_core::{EdgePlatform, ProcessLifecycleEvent, TickContext};
use mutop_ai::platform::MutopPlatform;
use serde_json::json;

#[test]
fn test_mutop_platform_metadata_and_mock_collection() {
    let mut platform = MutopPlatform::new_mock();
    let meta = platform.metadata();

    assert_eq!(meta.binary_name, "mutop-ai");
    assert!(meta.board_name.contains("LattePanda Mu Ultra"));
    assert_eq!(meta.mcp_scheme, "mutop");
    assert_eq!(meta.default_idle_baseline_w, 4.8);

    let ctx = TickContext {
        interval_ms: Some(200.0),
        marker: Some("bench_start".to_string()),
        events: vec![ProcessLifecycleEvent {
            event: "started".into(),
            pid: 4096,
            name: "llama-server.exe".to_string(),
            timestamp_unix_ms: 1730000000000,
        }],
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        tick_index: 0,
        elapsed_run_secs: 0.2,
    };

    let snapshot = platform.collect_snapshot(&ctx);
    assert_eq!(snapshot.host.total_ai_tops, 115);
    assert_eq!(snapshot.marker, Some("bench_start".to_string()));
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.events[0].name, "llama-server.exe");
    assert_eq!(snapshot.memory.installed_mop_mb, 16384);

    let procs = platform.current_tracked_processes(&snapshot);
    assert!(!procs.is_empty());
}

#[test]
fn test_mutop_platform_summary_and_diagnostics_extraction() {
    let mut platform = MutopPlatform::new_mock();
    let ctx = TickContext::default();
    let mut snapshot = platform.collect_snapshot(&ctx);

    // Case 1: When tracked_processes is None
    snapshot.tracked_processes = None;
    let sample = platform.extract_summary_sample(&snapshot, 50.0);
    assert!(sample.interval_secs > 0.0);
    assert!(sample.power_w.is_some());
    assert_eq!(
        sample.ram_available_gb,
        snapshot.memory.os_avail_phys_mb as f64 / 1024.0
    );
    assert_eq!(sample.cpu_big_pct, snapshot.cpu.lion_cove_p_load_pct);
    assert_eq!(sample.cpu_little_pct, snapshot.cpu.skymont_lpe_load_pct);
    assert!(sample.tracked_proc_name.is_none());
    assert!(sample.tracked_proc_cpu_pct.is_none());
    assert!(sample.tracked_proc_big_cpu_pct.is_none());
    assert!(sample.tracked_proc_little_cpu_pct.is_none());
    assert!(sample.tracked_proc_big_threads.is_none());
    assert!(sample.tracked_proc_little_threads.is_none());
    assert!(sample.tracked_proc_rss_mb.is_none());
    assert!(sample.tracked_proc_eco_qos.is_none());

    // Case 2: With tracked processes and threads
    let ctx_procs = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        ..Default::default()
    };
    let snap_procs = platform.collect_snapshot(&ctx_procs);
    let sample_procs = platform.extract_summary_sample(&snap_procs, 50.0);
    assert_eq!(
        sample_procs.tracked_proc_name.as_deref(),
        Some("llama-server.exe")
    );
    assert!(sample_procs.tracked_proc_cpu_pct.is_some());
    assert!(sample_procs.tracked_proc_big_cpu_pct.is_some());
    assert!(sample_procs.tracked_proc_little_cpu_pct.is_some());
    assert!(sample_procs.tracked_proc_big_threads.is_some());
    assert!(sample_procs.tracked_proc_little_threads.is_some());
    assert!(sample_procs.tracked_proc_rss_mb.is_some());
    assert_eq!(sample_procs.tracked_proc_eco_qos, Some(true));

    let diag = platform.extract_diagnostics(&snapshot);
    assert_eq!(diag.health_status, "Healthy");
    assert!(diag.thermal_headroom_c.is_some());
    assert!(diag.npu_status.contains("Intel(R) AI Boost"));

    // Case 3: Unprivileged scenario with no thermal sensors available
    let mut unpriv_snap = snapshot.clone();
    unpriv_snap.power_thermals.cpu_package_dts_c = None;
    unpriv_snap.power_thermals.acpi_thermal_zone_c = None;
    unpriv_snap.power_thermals.gpu_c = None;
    unpriv_snap.power_thermals.nvme_composite_c = None;
    unpriv_snap.power_thermals.thermal_status =
        Some("temperature not available (not elevated)".to_string());

    let unpriv_sample = platform.extract_summary_sample(&unpriv_snap, 50.0);
    assert_eq!(
        unpriv_sample.thermal_status.as_deref(),
        Some("temperature not available (not elevated)")
    );

    let unpriv_diag = platform.extract_diagnostics(&unpriv_snap);
    assert!(
        unpriv_diag.thermal_headroom_c.is_none(),
        "thermal_headroom_c must be None when no temperatures are available"
    );
    assert_eq!(
        unpriv_diag.thermal_status.as_deref(),
        Some("temperature not available (not elevated)")
    );

    // Case 4: Peak temperature selects the maximum across all zones
    let mut multi_temp_snap = snapshot.clone();
    multi_temp_snap.power_thermals.cpu_package_dts_c = Some(50.0);
    multi_temp_snap.power_thermals.gpu_c = Some(75.0);
    let multi_diag = platform.extract_diagnostics(&multi_temp_snap);
    assert_eq!(multi_diag.thermal_headroom_c, Some(25)); // (100 - 75).max(0)
}

#[test]
fn test_mutop_platform_oneshot_format() {
    let mut platform = MutopPlatform::new_mock();
    let ctx = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        ..Default::default()
    };
    let snapshot = platform.collect_snapshot(&ctx);

    let oneshot = platform.format_oneshot(&snapshot);
    assert!(
        oneshot.is_ascii(),
        "format_oneshot output must be pure 7-bit ASCII"
    );
    assert!(oneshot.contains("MUTOP-AI TELEMETRY SNAPSHOT"));
    assert!(oneshot.contains("LattePanda Mu Ultra"));
    assert!(oneshot.contains("115"));
    assert!(oneshot.contains("Lion Cove P"));
    assert!(oneshot.contains("Skymont LP-E"));
    assert!(oneshot.contains("Arc 140V GPU"));
    assert!(oneshot.contains("AI Boost NPU 4"));
    assert!(oneshot.contains("16 GB LPDDR5X-8533 MoP"));
    assert!(oneshot.contains("Thermals: CPU: 48.5C | GPU: 44.0C | NVMe: 41.0C | Fan: 2400 RPM"));

    // Fallback when CPU package DTS & ACPI thermal zone are None and Fan is None
    // Tier 1: Unprivileged explicitly displays "n/a (not elevated)"
    let mut fallback_snap = snapshot.clone();
    fallback_snap.host.privilege_tier =
        mutop_ai::snapshot::types::PrivilegeTier::Tier1Unprivileged;
    fallback_snap.power_thermals.cpu_package_dts_c = None;
    fallback_snap.power_thermals.acpi_thermal_zone_c = None;
    fallback_snap.power_thermals.cpu_fan_rpm = None;
    let fallback_oneshot = platform.format_oneshot(&fallback_snap);
    assert!(fallback_oneshot.contains("Thermals: CPU: n/a (not elevated) | GPU: 44.0C | NVMe: 41.0C | Fan: Auto"));

    // Tier 2: Elevated when sensors absent displays "-"
    let mut elevated_fallback = fallback_snap.clone();
    elevated_fallback.host.privilege_tier =
        mutop_ai::snapshot::types::PrivilegeTier::Tier2ElevatedAdmin;
    let elevated_oneshot = platform.format_oneshot(&elevated_fallback);
    assert!(elevated_oneshot.contains("Thermals: CPU: - | GPU: 44.0C | NVMe: 41.0C | Fan: Auto"));
}

#[test]
fn test_summary_report_ascii_and_thread_placement() {
    let mut platform = MutopPlatform::new_mock();
    let mut accumulator = edgetop_core::summary::SummaryAccumulator::default();

    // Sample 1: start marker
    let ctx1 = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        marker: Some("prefill-start".to_string()),
        ..Default::default()
    };
    let snap1 = platform.collect_snapshot(&ctx1);
    let sample1 = platform.extract_summary_sample(&snap1, 50.0);
    accumulator.record_sample(&sample1);

    // Sample 2: active generation with process telemetry
    let ctx2 = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        marker: None,
        ..Default::default()
    };
    let snap2 = platform.collect_snapshot(&ctx2);
    let sample2 = platform.extract_summary_sample(&snap2, 50.0);
    accumulator.record_sample(&sample2);

    // Sample 3: terminal marker
    let ctx3 = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        marker: Some("gen-end".to_string()),
        ..Default::default()
    };
    let snap3 = platform.collect_snapshot(&ctx3);
    let sample3 = platform.extract_summary_sample(&snap3, 50.0);
    accumulator.record_sample(&sample3);

    let report = accumulator.format_report(platform.metadata(), 50.0, 0.6);
    assert!(
        report.is_ascii(),
        "accumulator.format_report output must be pure 7-bit ASCII"
    );

    // Assert tracked process metrics rows in table
    assert!(report.contains("Tracked Proc CPU"));
    assert!(report.contains("  Proc P/Big Core CPU"));
    assert!(report.contains("  Proc LP-E/Little CPU"));
    assert!(report.contains("  Proc P/Big Threads"));
    assert!(report.contains("  Proc LP-E/Little Threads"));
    assert!(report.contains("  Proc Working Set"));

    // Assert marker interval summary and process thread placement line
    assert!(report.contains("MARKER PHASES & INTERVAL PROFILING:"));
    assert!(report.contains("\"prefill-start\" -> \"gen-end\""));
    assert!(report.contains("Process :"));
    assert!(report.contains("llama-server.exe"));
    assert!(report.contains("P/Big:"));
    assert!(report.contains("LP-E/Little:"));
    assert!(report.contains("[EcoQoS]"));
}

#[test]
fn test_mutop_cli_parsing_dispatch() {
    let platform = MutopPlatform::new_mock();
    let meta = platform.metadata();

    assert_eq!(
        parse_args(["mutop-ai", "-h"], meta).unwrap(),
        CliAction::PrintHelp
    );
    assert_eq!(
        parse_args(["mutop-ai", "-v"], meta).unwrap(),
        CliAction::PrintVersion
    );
    assert_eq!(
        parse_args(["mutop-ai", "--schema"], meta).unwrap(),
        CliAction::PrintSchema
    );

    match parse_args(["mutop-ai", "-j", "-p", "llama"], meta).unwrap() {
        CliAction::Json(opts) => {
            assert_eq!(opts.tracked_processes, vec!["llama".to_string()]);
        }
        _ => panic!("Expected Json action"),
    }

    match parse_args(["mutop-ai", "-1"], meta).unwrap() {
        CliAction::Oneshot(_) => {}
        _ => panic!("Expected Oneshot action"),
    }

    match parse_args(["mutop-ai", "-s", "250", "--summary"], meta).unwrap() {
        CliAction::Stream(opts) => {
            assert_eq!(opts.interval_ms, 250);
            assert!(opts.summary);
        }
        _ => panic!("Expected Stream action"),
    }

    // Verify automatic Stream action inference when -d / --summary is passed without -s
    match parse_args(["mutop-ai", "-d", "2", "--summary"], meta).unwrap() {
        CliAction::Stream(opts) => {
            assert_eq!(opts.duration_secs, Some(2));
            assert!(opts.summary);
            assert_eq!(opts.interval_ms, meta.default_stream_interval_ms);
        }
        _ => panic!("Expected inferred Stream action when -d and --summary are passed"),
    }

    match parse_args(["mutop-ai", "--mcp"], meta).unwrap() {
        CliAction::McpStdio(_) => {}
        _ => panic!("Expected McpStdio action"),
    }

    match parse_args(["mutop-ai", "--mcp-port", "8899"], meta).unwrap() {
        CliAction::McpHttp(addr, _) => {
            assert_eq!(addr, "0.0.0.0:8899");
        }
        _ => panic!("Expected McpHttp action"),
    }

    match parse_args(["mutop-ai"], meta).unwrap() {
        CliAction::RunTui(_) => {}
        _ => panic!("Expected RunTui action"),
    }
}

#[test]
fn test_mutop_mcp_jsonrpc_protocol() {
    let mut platform = MutopPlatform::new_mock();

    // 1. initialize
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let resp = handle_mcp_request(&mut platform, &init_req, None).unwrap();
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(resp["result"]["serverInfo"]["name"], "mutop-ai");

    // 2. tools/list
    let list_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let resp = handle_mcp_request(&mut platform, &list_req, None).unwrap();
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "get_ai_diagnostics"));
    assert!(tools.iter().any(|t| t["name"] == "get_telemetry_snapshot"));
    assert!(tools.iter().any(|t| t["name"] == "get_hardware_schema"));

    // 3. tools/call: get_telemetry_snapshot
    let call_snap = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "get_telemetry_snapshot",
            "arguments": {}
        }
    });
    let resp = handle_mcp_request(&mut platform, &call_snap, None).unwrap();
    assert_eq!(resp["result"]["content"][0]["type"], "text");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"total_ai_tops\": 115"));

    // 4. resources/read: mutop://telemetry/schema
    let read_schema = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "resources/read",
        "params": {
            "uri": "mutop://telemetry/schema"
        }
    });
    let resp = handle_mcp_request(&mut platform, &read_schema, None).unwrap();
    let schema_text = resp["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(schema_text.contains("timestamp_unix"));
}

#[test]
fn test_mutop_stream_execution() {
    let mut platform = MutopPlatform::new_mock();
    let opts = StreamOptions {
        interval_ms: 10,
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        duration_secs: None,
        summary: true,
        mark_file: None,
        active_threshold_pct: 50.0,
    };

    let mut out = Vec::new();
    let mut tick = 0;
    let accumulator = run_stream(&mut platform, &opts, &mut out, || {
        tick += 1;
        tick > 3
    });

    assert_eq!(accumulator.total_samples, 3);
    let out_str = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out_str.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in lines {
        let snap: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(snap["host"]["total_ai_tops"], 115);
    }
}

#[test]
fn test_summary_sample_preserves_slots_metrics() {
    let mut platform = MutopPlatform::new_mock();
    let ctx = TickContext::default();
    let mut snapshot = platform.collect_snapshot(&ctx);

    snapshot.npu.slots_metrics = Some(mutop_ai::snapshot::types::LlamaSlotsMetrics {
        prompt_tokens_per_sec: 120.5,
        generation_tokens_per_sec: 35.2,
        active_slots: 1,
        total_slots: 4,
        prompt_tokens_processed: 512,
        tokens_decoded: 128,
        completed_task_tokens: Some(640),
    });

    let sample = platform.extract_summary_sample(&snapshot, 50.0);
    assert_eq!(sample.prompt_tokens_processed, 512);
    assert_eq!(sample.tokens_decoded, 128);
    assert_eq!(sample.completed_task_tokens, Some(640));
    assert_eq!(sample.prompt_tps, Some(120.5));
    assert_eq!(sample.decode_tps, Some(35.2));
}

#[test]
fn test_calibrate_idle_baseline() {
    let mut platform = MutopPlatform::new_mock();
    assert_eq!(platform.collector_mut().idle_baseline_w, 4.8);
    let calibrated = platform.calibrate_idle_baseline(3, std::time::Duration::ZERO);
    assert_eq!(calibrated, Some(17.5));
    assert_eq!(platform.collector_mut().idle_baseline_w, 17.5);
}

#[test]
fn test_include_threads_omission_behavior() {
    let mut platform = MutopPlatform::new_mock();

    // Case 1: include_threads = false -> threads array omitted from serialized JSON
    let ctx_no_threads = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        include_threads: false,
        ..Default::default()
    };
    let snap_no_threads = platform.collect_snapshot(&ctx_no_threads);
    let json_no_threads = serde_json::to_string(&snap_no_threads).unwrap();
    let val_no_threads: serde_json::Value = serde_json::from_str(&json_no_threads).unwrap();
    let procs_no_threads = val_no_threads["tracked_processes"].as_array().unwrap();
    assert!(!procs_no_threads.is_empty());
    assert!(procs_no_threads[0].get("threads").is_none());

    // Case 2: include_threads = true -> threads array present and populated
    let ctx_with_threads = TickContext {
        tracked_patterns: vec!["llama".to_string()],
        include_threads: true,
        ..Default::default()
    };
    let snap_with_threads = platform.collect_snapshot(&ctx_with_threads);
    let json_with_threads = serde_json::to_string(&snap_with_threads).unwrap();
    let val_with_threads: serde_json::Value = serde_json::from_str(&json_with_threads).unwrap();
    let procs_with_threads = val_with_threads["tracked_processes"].as_array().unwrap();
    assert!(!procs_with_threads.is_empty());
    assert!(procs_with_threads[0].get("threads").is_some());
    assert!(!procs_with_threads[0]["threads"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn test_unprivileged_summary_report_thermals() {
    let mut platform = MutopPlatform::new_mock();
    let ctx = TickContext::default();
    let mut snapshot = platform.collect_snapshot(&ctx);

    snapshot.host.privilege_tier =
        mutop_ai::snapshot::types::PrivilegeTier::Tier1Unprivileged;
    snapshot.power_thermals.cpu_package_dts_c = None;
    snapshot.power_thermals.acpi_thermal_zone_c = None;
    snapshot.power_thermals.gpu_c = None;
    snapshot.power_thermals.nvme_composite_c = None;
    snapshot.power_thermals.cpu_fan_rpm = None;
    snapshot.power_thermals.thermal_status =
        mutop_ai::hardware::power::determine_thermal_status(
            snapshot.host.privilege_tier,
            snapshot.power_thermals.cpu_package_dts_c,
            snapshot.power_thermals.acpi_thermal_zone_c,
            snapshot.power_thermals.gpu_c,
            snapshot.power_thermals.nvme_composite_c,
        );
    assert_eq!(
        snapshot.power_thermals.thermal_status.as_deref(),
        Some("temperature not available (not elevated)")
    );

    let sample = platform.extract_summary_sample(&snapshot, 50.0);
    assert!(sample.temps.is_empty());
    assert_eq!(
        sample.thermal_status.as_deref(),
        Some("temperature not available (not elevated)")
    );

    let mut accumulator = edgetop_core::summary::SummaryAccumulator::default();
    accumulator.record_sample(&sample);
    let report = accumulator.format_report(platform.metadata(), 50.0, 0.2);

    assert!(report.is_ascii());
    assert!(report.contains("Thermals           : temperature not available (not elevated)"));
    assert!(!report.contains("Thermals:\n"));
}

#[test]
fn test_mcp_get_telemetry_snapshot_raw_thermals_filter() {
    let mut platform = MutopPlatform::new_mock();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "tools/call",
        "params": {
            "name": "get_telemetry_snapshot",
            "arguments": {
                "include_raw_thermals": false
            }
        }
    });

    let resp = handle_mcp_request(&mut platform, &req, None).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let snap: serde_json::Value = serde_json::from_str(text).unwrap();

    // Power rails and package temp must be preserved; raw per-core array is stripped
    assert!(snap["power_thermals"].get("package_w").is_some());
    assert!(snap["power_thermals"].get("core_pp0_w").is_some());
    assert!(snap["power_thermals"].get("dram_w").is_some());
    assert!(snap["power_thermals"].get("cpu_package_dts_c").is_some());
    assert!(snap["power_thermals"].get("cpu_core_dts_c").is_none());
}
