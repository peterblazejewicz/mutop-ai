// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

pub mod cpu;
pub mod diagnostics;
pub mod dxgi;
pub mod facts;
pub mod gpu;
pub mod memory;
pub mod network;
pub mod npu;
pub mod pdh;
pub mod power;
pub mod processes;
pub mod storage;
pub mod timing;

use std::time::{SystemTime, UNIX_EPOCH};

pub use cpu::{collect_cpu_snapshot, CpuCollectorState};
pub use diagnostics::evaluate_diagnostics;
pub use facts::get_host_info;
pub use gpu::collect_gpu_snapshot;
pub use memory::collect_memory_snapshot;
pub use network::{collect_network_snapshot, NetworkCollectorState};
pub use npu::collect_npu_snapshot;
pub use pdh::win_pdh::PdhTelemetryState;
pub use power::{collect_power_thermal_snapshot, PowerCollectorState};
pub use processes::{collect_tracked_processes, ProcessesCollectorState};
pub use storage::{collect_storage_snapshot, StorageCollectorState};
pub use timing::collect_timing_snapshot;

use crate::snapshot::types::{CoreMicroarch, HostInfo, MutopSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectorMode {
    Live,
    Mock,
}

pub struct HardwareCollector {
    pub mode: CollectorMode,
    pub host_info: HostInfo,
    pub cpu_state: CpuCollectorState,
    pub network_state: NetworkCollectorState,
    pub storage_state: StorageCollectorState,
    pub power_state: PowerCollectorState,
    pub processes_state: ProcessesCollectorState,
    pub adapters: Vec<crate::hardware::dxgi::AdapterInfo>,
    pub pdh_state: Option<PdhTelemetryState>,
    pub idle_baseline_w: f64,
    pub last_tick_instant: Option<std::time::Instant>,
}

impl HardwareCollector {
    pub fn new() -> Self {
        let is_mock = false;
        #[allow(unused_mut)]
        let mut cpu_state = CpuCollectorState::default();
        #[cfg(windows)]
        {
            cpu_state.topology = cpu::discover_windows_core_topology(8);
        }
        let adapters = crate::hardware::dxgi::enumerate_dxgi_adapters();
        let host_info = get_host_info(is_mock);
        Self {
            mode: CollectorMode::Live,
            host_info,
            cpu_state,
            network_state: NetworkCollectorState::default(),
            storage_state: StorageCollectorState::default(),
            power_state: PowerCollectorState::default(),
            processes_state: ProcessesCollectorState::default(),
            adapters,
            pdh_state: PdhTelemetryState::new(),
            idle_baseline_w: 4.8,
            last_tick_instant: None,
        }
    }

    pub fn new_mock() -> Self {
        let is_mock = true;
        let cpu_state = CpuCollectorState {
            topology: vec![
                CoreMicroarch::LionCoveP,
                CoreMicroarch::LionCoveP,
                CoreMicroarch::LionCoveP,
                CoreMicroarch::LionCoveP,
                CoreMicroarch::SkymontLpE,
                CoreMicroarch::SkymontLpE,
                CoreMicroarch::SkymontLpE,
                CoreMicroarch::SkymontLpE,
            ],
            ..Default::default()
        };
        let host_info = get_host_info(is_mock);
        Self {
            mode: CollectorMode::Mock,
            host_info,
            cpu_state,
            network_state: NetworkCollectorState::default(),
            storage_state: StorageCollectorState::default(),
            power_state: PowerCollectorState::default(),
            processes_state: ProcessesCollectorState::default(),
            adapters: Vec::new(),
            pdh_state: None,
            idle_baseline_w: 4.8,
            last_tick_instant: None,
        }
    }

    pub fn collect_snapshot(
        &mut self,
        marker: Option<String>,
        tracked_patterns: &[String],
        include_threads: bool,
    ) -> MutopSnapshot {
        let is_mock = self.mode == CollectorMode::Mock;
        let now = SystemTime::now();
        let dur = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        let timestamp_unix = dur.as_secs();
        let timestamp_unix_ms = dur.as_millis() as u64;

        let interval_ms = self
            .last_tick_instant
            .map(|inst| inst.elapsed().as_secs_f64() * 1000.0);
        self.last_tick_instant = Some(std::time::Instant::now());

        if !is_mock {
            if let Some(ref pdh) = self.pdh_state {
                pdh.collect();
            }
        }

        let host = self.host_info.clone();
        let cpu = collect_cpu_snapshot(&mut self.cpu_state, is_mock);
        let engine_items = if !is_mock {
            self.pdh_state
                .as_ref()
                .map(|p| p.read_array("gpu_engines"))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let npu = collect_npu_snapshot(
            &self.adapters,
            &engine_items,
            self.pdh_state.as_ref(),
            is_mock,
        );
        let gpu = collect_gpu_snapshot(
            &self.adapters,
            &engine_items,
            self.pdh_state.as_ref(),
            is_mock,
        );
        let memory = collect_memory_snapshot(
            gpu.shared_memory_used_mb,
            npu.shared_memory_used_mb,
            is_mock,
        );
        let power_thermals = collect_power_thermal_snapshot(
            &mut self.power_state,
            self.pdh_state.as_ref(),
            self.idle_baseline_w,
            is_mock,
        );
        let storage =
            collect_storage_snapshot(&mut self.storage_state, self.pdh_state.as_ref(), is_mock);
        let network = collect_network_snapshot(&mut self.network_state, is_mock);
        let timing = collect_timing_snapshot(is_mock);

        let tracked_processes = if !tracked_patterns.is_empty() || is_mock {
            let procs = collect_tracked_processes(
                &mut self.processes_state,
                &self.adapters,
                &self.cpu_state.topology,
                self.pdh_state.as_ref(),
                tracked_patterns,
                include_threads,
                is_mock,
            );
            if procs.is_empty() {
                None
            } else {
                Some(procs)
            }
        } else {
            None
        };

        let is_marker_active = marker.is_some();
        let ai_diagnostics = evaluate_diagnostics(
            &cpu,
            &memory,
            &gpu,
            &power_thermals,
            &storage,
            &timing,
            tracked_processes.as_deref(),
            is_marker_active,
        );

        MutopSnapshot {
            timestamp_unix,
            timestamp_unix_ms,
            interval_ms,
            marker,
            host,
            cpu,
            memory,
            gpu,
            npu,
            power_thermals,
            storage,
            network,
            timing,
            ai_diagnostics,
            tracked_processes,
            events: Vec::new(),
        }
    }
}

impl Default for HardwareCollector {
    fn default() -> Self {
        Self::new()
    }
}
