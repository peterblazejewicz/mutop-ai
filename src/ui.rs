// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table},
    Frame,
};

use crate::snapshot::types::{
    DiagnosticStatus, EffectivePowerMode, MutopSnapshot, NpuActivityStatus, ProcessAffinityProfile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSortMode {
    CpuDesc,
    MemoryDesc,
    PidAsc,
    NameAsc,
}

pub struct UiState {
    pub process_sort: ProcessSortMode,
    pub process_filter: String,
    pub filter_mode: bool,
    pub scroll_offset: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            process_sort: ProcessSortMode::CpuDesc,
            process_filter: String::new(),
            filter_mode: false,
            scroll_offset: 0,
        }
    }
}

pub fn render_ui(frame: &mut Frame, snapshot: &MutopSnapshot, ui_state: &UiState) {
    let size = frame.area();

    // 5-Row Main Vertical Layout matching ventop-ai
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(12), // Top Grid (CPU & Dual Accelerators)
            Constraint::Length(6),  // Mid Grid (Memory/Storage & Power/Thermals/Net)
            Constraint::Min(8),     // Process table
            Constraint::Length(1),  // Footer status / key help
        ])
        .split(size);

    render_header(frame, chunks[0], snapshot);
    render_top_grid(frame, chunks[1], snapshot);
    render_mid_grid(frame, chunks[2], snapshot);
    render_process_table(frame, chunks[3], snapshot, ui_state);
    render_footer(frame, chunks[4]);
}

fn render_header(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let uptime = snapshot.timing.unbiased_uptime_sec as u64;
    let hours = uptime / 3600;
    let mins = (uptime % 3600) / 60;
    let secs = uptime % 60;
    let compact = area.width < 122;

    let pwr_mode = match snapshot.power_thermals.effective_power_mode {
        EffectivePowerMode::BestPerformance => "Performance",
        EffectivePowerMode::BestPowerEfficiency => "Efficiency",
        EffectivePowerMode::Balanced => "Balanced",
        EffectivePowerMode::BatterySaver => "BatterySaver",
        EffectivePowerMode::Unknown => "Unknown",
    };

    let mut spans = vec![
        Span::styled(
            " MUTOP-AI ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if compact {
                " LP Mu Ultra "
            } else {
                "  LattePanda Mu Ultra "
            },
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if compact {
                "[Core Ultra 7 256V]"
            } else {
                "[Intel Core Ultra 7 256V + Arc 140V + NPU 4]"
            },
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" | "),
        Span::styled(
            format!("Uptime: {:02}:{:02}:{:02}", hours, mins, secs),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" | "),
        Span::styled(
            format!(
                "Power: {:5.2}W ({})",
                snapshot.power_thermals.package_w, pwr_mode
            ),
            Style::default().fg(Color::Cyan),
        ),
    ];

    if area.width >= 102 {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            if compact {
                "115 TOPS AI"
            } else {
                "115 TOPS EDGE AI"
            },
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let header_text = vec![Line::from(spans)];
    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, area);
}

fn render_top_grid(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(42), // CPU & Clusters
            Constraint::Percentage(58), // Dual Accelerators (NPU 4 + Arc 140V)
        ])
        .split(area);

    render_cpu_panel(frame, top_cols[0], snapshot);
    render_accelerators_panel(frame, top_cols[1], snapshot);
}

fn render_cpu_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let total_cpu = snapshot.cpu.total_load_pct;
    let inner_width = area.width.saturating_sub(2) as usize;
    let compact = inner_width < 54;
    let mut lines = Vec::new();

    // Total CPU bar matching ventop-ai (# and -)
    let bar_width: usize = if inner_width < 44 { 16 } else { 20 };
    let filled = ((total_cpu / 100.0) * bar_width as f32).round() as usize;
    let empty = bar_width.saturating_sub(filled);
    let bar = format!("[{}{}]", "#".repeat(filled), "-".repeat(empty));

    lines.push(Line::from(vec![
        Span::styled(
            "Total CPU: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:5.1}% ", total_cpu),
            Style::default().fg(if total_cpu > 80.0 {
                Color::Red
            } else {
                Color::Green
            }),
        ),
        Span::styled(bar, Style::default().fg(Color::Cyan)),
    ]));

    // Lion Cove P-core cluster (cores 0..4)
    let p_load = snapshot.cpu.lion_cove_p_load_pct;
    let p_cur_mhz = if !snapshot.cpu.per_core_current_mhz.is_empty() {
        let sum: u32 = snapshot.cpu.per_core_current_mhz
            [..4.min(snapshot.cpu.per_core_current_mhz.len())]
            .iter()
            .sum();
        sum / 4
    } else {
        0
    };
    let p_max_mhz = snapshot
        .cpu
        .per_core_max_mhz
        .first()
        .copied()
        .unwrap_or(2200);

    let p_label = if inner_width < 42 {
        "LionCove(P) : "
    } else {
        "LionCove (P) : "
    };
    let p_load_str = if compact {
        format!("{:3.0}%", p_load)
    } else {
        format!("{:3.0}% load", p_load)
    };
    let p_freq_str = format!("{:4} MHz", p_cur_mhz);
    let p_range_str = format!(" [max {} MHz, C0-C3]", p_max_mhz);

    lines.push(Line::from(vec![
        Span::styled(p_label, Style::default().fg(Color::LightYellow)),
        Span::styled(
            p_load_str,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" @ "),
        Span::styled(
            p_freq_str,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(p_range_str, Style::default().fg(Color::DarkGray)),
    ]));

    // Skymont LP-E core cluster (cores 4..8)
    let e_load = snapshot.cpu.skymont_lpe_load_pct;
    let e_cur_mhz = if snapshot.cpu.per_core_current_mhz.len() >= 8 {
        let sum: u32 = snapshot.cpu.per_core_current_mhz[4..8].iter().sum();
        sum / 4
    } else {
        0
    };
    let e_max_mhz = snapshot
        .cpu
        .per_core_max_mhz
        .get(4)
        .copied()
        .unwrap_or(2200);

    let e_label = if inner_width < 42 {
        "Skymont(LPE): "
    } else {
        "Skymont (LPE): "
    };
    let e_load_str = if compact {
        format!("{:3.0}%", e_load)
    } else {
        format!("{:3.0}% load", e_load)
    };
    let e_freq_str = format!("{:4} MHz", e_cur_mhz);
    let e_range_str = format!(" [max {} MHz, C4-C7]", e_max_mhz);

    lines.push(Line::from(vec![
        Span::styled(e_label, Style::default().fg(Color::LightGreen)),
        Span::styled(
            e_load_str,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" @ "),
        Span::styled(
            e_freq_str,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(e_range_str, Style::default().fg(Color::DarkGray)),
    ]));

    // Conserved & timer resolution line
    lines.push(Line::from(vec![
        Span::styled("Tick: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(
                "U:{:3.0}% S:{:3.0}% Idl:{:3.0}%",
                snapshot.cpu.user_pct, snapshot.cpu.system_pct, snapshot.cpu.idle_pct
            ),
            Style::default().fg(Color::White),
        ),
        Span::styled(" | Timer: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:5.2}ms", snapshot.timing.timer_resolution.current_ms),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    // Divider
    lines.push(Line::from(Span::styled(
        "─".repeat(inner_width.max(10)),
        Style::default().fg(Color::DarkGray),
    )));

    // Per-core load % grouped by cluster
    let mut p_spans = vec![Span::styled(
        if inner_width < 45 {
            "P-Cores (0-3): "
        } else {
            "LionCove (C0-C3): "
        },
        Style::default().fg(Color::LightYellow),
    )];
    for i in 0..4 {
        let u = snapshot.cpu.per_core_pct.get(i).copied().unwrap_or(0.0);
        let col = if u > 80.0 {
            Color::Red
        } else if u > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        p_spans.push(Span::styled(
            format!("C{}:{:3.0}% ", i, u),
            Style::default().fg(col),
        ));
    }
    lines.push(Line::from(p_spans));

    let mut e_spans = vec![Span::styled(
        if inner_width < 45 {
            "LP-E    (4-7): "
        } else {
            "Skymont  (C4-C7): "
        },
        Style::default().fg(Color::LightGreen),
    )];
    for i in 4..8 {
        let u = snapshot.cpu.per_core_pct.get(i).copied().unwrap_or(0.0);
        let col = if u > 80.0 {
            Color::Red
        } else if u > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        e_spans.push(Span::styled(
            format!("C{}:{:3.0}% ", i, u),
            Style::default().fg(col),
        ));
    }
    lines.push(Line::from(e_spans));

    // Per-core frequency (MHz)
    let mut p_freq_spans = vec![Span::styled(
        "P-Freq (MHz)    : ",
        Style::default().fg(Color::DarkGray),
    )];
    for i in 0..4 {
        let mhz = snapshot
            .cpu
            .per_core_current_mhz
            .get(i)
            .copied()
            .unwrap_or(0);
        p_freq_spans.push(Span::styled(
            format!("C{}:{:4} ", i, mhz),
            Style::default().fg(Color::White),
        ));
    }
    lines.push(Line::from(p_freq_spans));

    let mut e_freq_spans = vec![Span::styled(
        "E-Freq (MHz)    : ",
        Style::default().fg(Color::DarkGray),
    )];
    for i in 4..8 {
        let mhz = snapshot
            .cpu
            .per_core_current_mhz
            .get(i)
            .copied()
            .unwrap_or(0);
        e_freq_spans.push(Span::styled(
            format!("C{}:{:4} ", i, mhz),
            Style::default().fg(Color::White),
        ));
    }
    lines.push(Line::from(e_freq_spans));

    let cpu_block = Paragraph::new(lines).block(
        Block::default()
            .title(" CPU Subsystem (8 Cores) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(cpu_block, area);
}

fn render_accelerators_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let mut lines = Vec::new();

    // 1. THE AI NPU Header matching ventop-ai (◆ THE AI BRAIN:)
    let (npu_act_str, npu_act_col) = match snapshot.npu.activity_status {
        NpuActivityStatus::ActivePromptProcessing => ("Prompt (PP)", Color::LightYellow),
        NpuActivityStatus::ActiveTokenGeneration => ("Gen (TG)", Color::LightGreen),
        NpuActivityStatus::ActiveCompute => ("Active Compute", Color::LightGreen),
        NpuActivityStatus::Idle => ("Idle", Color::DarkGray),
    };

    lines.push(Line::from(vec![
        Span::styled(
            "◆ THE AI NPU: ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ({} TOPS)", snapshot.npu.name, snapshot.npu.npu_tops),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled("MCDM", Style::default().fg(Color::Cyan)),
        Span::raw(" ["),
        Span::styled(
            npu_act_str,
            Style::default()
                .fg(npu_act_col)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("]"),
    ]));

    // NPU Utilization Bar & Shared Memory
    let npu_bar_w: usize = if inner_width < 50 { 10 } else { 16 };
    let npu_filled = ((snapshot.npu.utilization_pct / 100.0) * npu_bar_w as f32).round() as usize;
    let npu_empty = npu_bar_w.saturating_sub(npu_filled);
    let npu_bar = format!("[{}{}]", "#".repeat(npu_filled), "-".repeat(npu_empty));

    lines.push(Line::from(vec![
        Span::styled("  Util    : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:5.1}% ", snapshot.npu.utilization_pct),
            Style::default().fg(Color::White),
        ),
        Span::styled(npu_bar, Style::default().fg(Color::Magenta)),
        Span::styled(
            if snapshot.npu.current_freq_mhz == 0 {
                " | Freq: Auto MHz".to_string()
            } else {
                format!(" | Freq: {:4} MHz", snapshot.npu.current_freq_mhz)
            },
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(
                " | Shared: {:4} / {:4} MB",
                snapshot.npu.shared_memory_used_mb, snapshot.npu.shared_memory_budget_mb
            ),
            Style::default().fg(Color::LightMagenta),
        ),
    ]));

    // llama-server /slots or AI Offload status
    if let Some(ref slots) = snapshot.npu.slots_metrics {
        lines.push(Line::from(vec![
            Span::styled("  llama   : ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("⚡ {:.1} tok/s", slots.generation_tokens_per_sec),
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" | Prefill: {:.0} T/s", slots.prompt_tokens_per_sec),
                Style::default().fg(Color::LightYellow),
            ),
            Span::styled(
                format!(" | Slots: {}/{}", slots.active_slots, slots.total_slots),
                Style::default().fg(Color::Cyan),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  llama   : ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if snapshot.network.loopback.llama_server_listening {
                    ":8080 listening | Slots Idle | Zero-Copy FastRPC/MCDM"
                } else {
                    "Offline (no /slots on :8080) | MCDM compute queue ready"
                },
                Style::default().fg(if snapshot.network.loopback.llama_server_listening {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
    }

    // Divider
    lines.push(Line::from(Span::styled(
        "─".repeat(inner_width.max(10)),
        Style::default().fg(Color::DarkGray),
    )));

    // 2. THE AI GPU Header matching ventop-ai (◆ THE ACTION BRAIN:)
    let gpu_freq_str = if snapshot.gpu.clock_mhz > 0 {
        format!("{:4} MHz", snapshot.gpu.clock_mhz)
    } else {
        "Auto MHz".to_string()
    };

    lines.push(Line::from(vec![
        Span::styled(
            "◆ THE AI GPU: ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "{} ({} TOPS, 8 Xe2-LPG)",
                snapshot.gpu.name, snapshot.gpu.gpu_tops
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(gpu_freq_str, Style::default().fg(Color::White)),
        Span::styled(
            format!(" | Busy: {:5.1}%", snapshot.gpu.total_utilization_pct),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    // GPU Engines
    lines.push(Line::from(vec![
        Span::styled("  Engines : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("3D:{:5.1}% ", snapshot.gpu.render_3d_pct),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("| Compute:{:5.1}% ", snapshot.gpu.compute_pct),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!("| Copy:{:5.1}% ", snapshot.gpu.copy_pct),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("| Decode:{:5.1}%", snapshot.gpu.video_decode_pct),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // GPU Shared VRAM & Power & Thermals
    let temp_str = snapshot
        .gpu
        .temperature_c
        .map(|t| format!("{:.0}°C", t))
        .unwrap_or_else(|| "-".to_string());
    let pwr_str = snapshot
        .gpu
        .power_w
        .map(|p| format!("{:.1}W", p))
        .unwrap_or_else(|| "-".to_string());

    lines.push(Line::from(vec![
        Span::styled("  VRAM    : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(
                "{:4} / {:4} MB",
                snapshot.gpu.shared_memory_used_mb, snapshot.gpu.shared_memory_budget_mb
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" | Power: {}", pwr_str),
            Style::default().fg(Color::LightRed),
        ),
        Span::styled(
            format!(" | Temp: {}", temp_str),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            if snapshot.gpu.throttling.power_limited {
                " | PWR-LIMIT"
            } else {
                " | Throt: None"
            },
            Style::default().fg(if snapshot.gpu.throttling.power_limited {
                Color::Red
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    // Active workloads
    lines.push(Line::from(vec![
        Span::styled("  Workload: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "DirectML / OpenVINO / Vulkan / SYCL Compute Queue",
            Style::default().fg(Color::Cyan),
        ),
    ]));

    let accelerators_block = Paragraph::new(lines).block(
        Block::default()
            .title(" Dual-Accelerator Telemetry (NPU 4 + Arc 140V) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Magenta)),
    );
    frame.render_widget(accelerators_block, area);
}

fn render_mid_grid(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Memory & Storage
            Constraint::Percentage(50), // Power, Thermals & Network
        ])
        .split(area);

    render_memory_storage_panel(frame, cols[0], snapshot);
    render_power_thermals_network_panel(frame, cols[1], snapshot);
}

fn render_memory_storage_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let total_ram_gb = snapshot.memory.installed_mop_mb as f64 / 1024.0;
    let used_ram_gb = snapshot.memory.in_use_phys_mb as f64 / 1024.0;
    let avail_ram_gb = snapshot.memory.os_avail_phys_mb as f64 / 1024.0;
    let ram_pct = (used_ram_gb / total_ram_gb.max(0.1)) * 100.0;

    let inner_width = area.width.saturating_sub(2) as usize;
    let bar_width: usize = if inner_width < 52 { 10 } else { 16 };
    let filled = ((ram_pct / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);
    let bar = format!("[{}{}]", "#".repeat(filled), "-".repeat(empty));

    let ram_val_str = format!(
        "{:5.2} / {:5.2} GB ({:4.1}%) ",
        used_ram_gb, total_ram_gb, ram_pct
    );

    let mem_lines = vec![
        Line::from(vec![
            Span::styled(
                "LPDDR5X RAM: ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(ram_val_str, Style::default().fg(Color::White)),
            Span::styled(bar, Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("Available  : ", Style::default().fg(Color::LightGreen)),
            Span::styled(
                format!("{:5.2} GB", avail_ram_gb),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " | CPU: {:4}MB | Arc: {:4}MB | NPU: {:4}MB",
                    snapshot.memory.pure_cpu_in_use_mb,
                    snapshot.memory.arc_gpu_shared_mb,
                    snapshot.memory.npu_shared_mb
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Mem Xfer   : ", Style::default().fg(Color::LightCyan)),
            Span::styled(
                format!(
                    "Commit: {:5}/{:5}MB | BIOS Aperture: {:3}MB",
                    snapshot.memory.commit_total_mb,
                    snapshot.memory.commit_limit_mb,
                    snapshot.memory.bios_stolen_aperture_mb
                ),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("NVMe Disk  : ", Style::default().fg(Color::Cyan)),
            Span::styled(
                {
                    let pcie_tag =
                        if snapshot.storage.current_gen > 0 && snapshot.storage.current_lanes > 0 {
                            format!(
                                "[Gen{}x{}] ",
                                snapshot.storage.current_gen, snapshot.storage.current_lanes
                            )
                        } else {
                            String::new()
                        };
                    format!(
                        "C:\\ {:.1}/{:.1}GB ({:.0}%) {}{}",
                        snapshot.storage.capacity_gb - snapshot.storage.free_gb,
                        snapshot.storage.capacity_gb,
                        snapshot.storage.used_pct,
                        pcie_tag,
                        if snapshot.storage.is_link_degraded {
                            "(Degraded) "
                        } else {
                            ""
                        }
                    )
                },
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(
                    "| R:{:4.1} W:{:4.1}MB/s (Util:{:2.0}%)",
                    snapshot.storage.read_mb_s,
                    snapshot.storage.write_mb_s,
                    snapshot.storage.busy_pct
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let mem_block = Paragraph::new(mem_lines).block(
        Block::default()
            .title(" Memory, Transfers & Built-in Storage ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Green)),
    );
    frame.render_widget(mem_block, area);
}

fn render_power_thermals_network_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let mut lines = Vec::new();

    // Package power & rails
    lines.push(Line::from(vec![
        Span::styled(
            "Package Pwr: ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:5.2} W ", snapshot.power_thermals.package_w),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "[PP0:{:5.2}W PP1:{:5.2}W DRAM:{:5.2}W] | Base: {:4.1}W",
                snapshot.power_thermals.core_pp0_w,
                snapshot.power_thermals.gpu_pp1_w,
                snapshot.power_thermals.dram_w,
                snapshot.power_thermals.idle_baseline_w
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Thermals & fan & AI health
    let acpi_t = snapshot
        .power_thermals
        .acpi_thermal_zone_c
        .map(|t| format!("{:.0}°C", t))
        .unwrap_or_else(|| "-".to_string());
    let gpu_t = snapshot
        .power_thermals
        .gpu_c
        .map(|t| format!("{:.0}°C", t))
        .unwrap_or_else(|| "-".to_string());
    let nvme_t = snapshot
        .power_thermals
        .nvme_composite_c
        .map(|t| format!("{:.0}°C", t))
        .unwrap_or_else(|| "-".to_string());
    let fan_rpm = snapshot
        .power_thermals
        .cpu_fan_rpm
        .map(|r| format!("{} RPM", r))
        .unwrap_or_else(|| "-".to_string());

    let (ai_status_str, ai_status_col) = match snapshot.ai_diagnostics.status {
        DiagnosticStatus::Healthy => ("HEALTHY", Color::LightGreen),
        DiagnosticStatus::Degraded => ("DEGRADED", Color::Yellow),
        DiagnosticStatus::Critical => ("CRITICAL", Color::LightRed),
    };

    lines.push(Line::from(vec![
        Span::styled("Thermals   : ", Style::default().fg(Color::LightRed)),
        Span::styled(
            format!("ACPI:{} GPU:{} NVMe:{} ", acpi_t, gpu_t, nvme_t),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("| Fan: {} ", fan_rpm),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("| AI: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            ai_status_str,
            Style::default()
                .fg(ai_status_col)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Ethernet (find first non-virtual physical adapter)
    let eth_iface = snapshot
        .network
        .interfaces
        .iter()
        .find(|i| i.name.contains("Ethernet") || i.name.contains("I226"));
    if let Some(eth) = eth_iface {
        let mut eth_clean = eth.name.as_str();
        if let Some(idx) = eth_clean.find("-WFP") {
            eth_clean = &eth_clean[..idx];
        }
        if let Some(idx) = eth_clean.find("-QoS") {
            eth_clean = &eth_clean[..idx];
        }
        if let Some(idx) = eth_clean.find("-Native") {
            eth_clean = &eth_clean[..idx];
        }
        for prefix in &[
            "Intel(R) Ethernet Controller ",
            "Intel(R) Ethernet Connection ",
            "Intel(R) Gigabit Network Connection ",
            "Intel(R) ",
            "Realtek PCIe GbE Family Controller",
            "Realtek PCIe 2.5GbE Family Controller",
            "Realtek PCIe ",
            "Realtek ",
        ] {
            if let Some(stripped) = eth_clean.strip_prefix(prefix) {
                eth_clean = stripped;
                break;
            }
        }
        let eth_clean = eth_clean.trim();
        let eth_display = if eth_clean.len() > 20 {
            &eth_clean[..20]
        } else {
            eth_clean
        };

        lines.push(Line::from(vec![
            Span::styled("Ethernet   : ", Style::default().fg(Color::LightBlue)),
            Span::styled(
                format!(
                    "{} ({} {}Mbps) ",
                    eth_display,
                    eth.oper_status.to_lowercase(),
                    eth.link_speed_mbps
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(
                    "| RX:{:5.2}MB/s TX:{:5.2}MB/s",
                    eth.rx_mbps / 8.0,
                    eth.tx_mbps / 8.0
                ),
                Style::default().fg(Color::Cyan),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("Ethernet   : ", Style::default().fg(Color::LightBlue)),
            Span::styled("None", Style::default().fg(Color::DarkGray)),
        ]));
    }

    // WLAN / Wi-Fi 7 BE200
    if let Some(ref wifi) = snapshot.network.wifi7 {
        let mlo_str = if wifi.is_mlo { "MLO" } else { "Single" };
        let wifi_iface = snapshot.network.interfaces.iter().find(|i| {
            i.name.contains("Wi-Fi")
                || i.name.contains("Wireless")
                || i.name.contains("BE20")
                || i.name.contains("WLAN")
        });
        let (wifi_rx_mbs, wifi_tx_mbs) = if let Some(iface) = wifi_iface {
            (iface.rx_mbps / 8.0, iface.tx_mbps / 8.0)
        } else {
            (0.0, 0.0)
        };

        lines.push(Line::from(vec![
            Span::styled("WLAN       : ", Style::default().fg(Color::LightGreen)),
            Span::styled(
                format!(
                    "Wi-Fi 7 (up {:3}dBm Q:{:2}% {:4}Mbps {}) ",
                    wifi.rssi_dbm, wifi.signal_quality_pct, wifi.rx_rate_mbps, mlo_str
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("| RX:{:5.2}MB/s TX:{:5.2}MB/s", wifi_rx_mbs, wifi_tx_mbs),
                Style::default().fg(Color::Cyan),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("WLAN       : ", Style::default().fg(Color::LightGreen)),
            Span::styled("None", Style::default().fg(Color::DarkGray)),
        ]));
    }

    let pwr_block = Paragraph::new(lines).block(
        Block::default()
            .title(" Power, Thermals & Network (ETH / Wi-Fi) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(pwr_block, area);
}

fn render_process_table(
    frame: &mut Frame,
    area: Rect,
    snapshot: &MutopSnapshot,
    ui_state: &UiState,
) {
    let header_cells = [
        "PID", "Name", "CPU %", "P-Core %", "LP-E %", "WS (MB)", "GPU (MB)", "NPU (MB)", "Core",
        "Th", "EcoQoS",
    ]
    .iter()
    .map(|h| {
        Span::styled(
            *h,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let empty_vec = Vec::new();
    let procs = snapshot.tracked_processes.as_ref().unwrap_or(&empty_vec);

    let matching_count = procs
        .iter()
        .filter(|p| {
            if ui_state.process_filter.is_empty() {
                true
            } else {
                p.name
                    .to_lowercase()
                    .contains(&ui_state.process_filter.to_lowercase())
            }
        })
        .count();

    let mut filtered_procs: Vec<&ProcessAffinityProfile> = procs
        .iter()
        .filter(|p| {
            if ui_state.process_filter.is_empty() {
                true
            } else {
                p.name
                    .to_lowercase()
                    .contains(&ui_state.process_filter.to_lowercase())
            }
        })
        .collect();

    match ui_state.process_sort {
        ProcessSortMode::CpuDesc => filtered_procs.sort_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        ProcessSortMode::MemoryDesc => {
            filtered_procs.sort_by_key(|p| std::cmp::Reverse(p.cpu_working_set_mb))
        }
        ProcessSortMode::PidAsc => filtered_procs.sort_by_key(|p| p.pid),
        ProcessSortMode::NameAsc => filtered_procs.sort_by(|a, b| a.name.cmp(&b.name)),
    }

    let rows: Vec<Row> = filtered_procs
        .iter()
        .skip(ui_state.scroll_offset)
        .map(|p| {
            let is_ai = p.name.contains("llama")
                || p.name.contains("device-commander")
                || p.name.contains("mutop")
                || p.name.contains("python");
            let name_style = if is_ai {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let primary_core = p.threads.first().map(|t| t.cpu_core).unwrap_or(0);
            let core_label = if primary_core < 4 {
                format!("C{} (Lion)", primary_core)
            } else {
                format!("C{} (Skym)", primary_core)
            };
            let core_color = if primary_core < 4 {
                Color::Yellow
            } else {
                Color::DarkGray
            };

            Row::new(vec![
                Span::raw(p.pid.to_string()),
                Span::styled(&p.name, name_style),
                Span::styled(
                    format!("{:5.1}", p.cpu_pct),
                    if p.cpu_pct > 50.0 {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    format!("{:5.1}", p.lion_cove_p_cpu_pct),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("{:5.1}", p.skymont_lpe_cpu_pct),
                    Style::default().fg(Color::LightGreen),
                ),
                Span::raw(p.cpu_working_set_mb.to_string()),
                Span::raw(p.gpu_shared_mb.to_string()),
                Span::raw(p.npu_shared_mb.to_string()),
                Span::styled(core_label, Style::default().fg(core_color)),
                Span::raw(format!(
                    "P:{}/E:{}",
                    p.lion_cove_p_threads, p.skymont_lpe_threads
                )),
                Span::styled(
                    if p.is_eco_qos { "True" } else { "False" },
                    if p.is_eco_qos {
                        Style::default().fg(Color::LightGreen)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
            ])
        })
        .collect();

    let title = format_process_table_title(ui_state, matching_count);

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(22),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(table, area);
}

pub fn format_process_table_title(ui_state: &UiState, total_matching: usize) -> String {
    if ui_state.filter_mode {
        format!(" Process List [Filter: {}_] ", ui_state.process_filter)
    } else if !ui_state.process_filter.is_empty() && ui_state.scroll_offset > 0 {
        format!(
            " Process List [Filter: {}] [Offset: +{}/{}] ",
            ui_state.process_filter, ui_state.scroll_offset, total_matching
        )
    } else if !ui_state.process_filter.is_empty() {
        format!(" Process List [Filter: {}] ", ui_state.process_filter)
    } else if ui_state.scroll_offset > 0 {
        format!(
            " Process List [Offset: +{}/{}] ",
            ui_state.scroll_offset, total_matching
        )
    } else {
        " Process List ".to_string()
    }
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer_text = Line::from(vec![
        Span::styled(
            " Q / Esc",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Quit | "),
        Span::styled(
            "▲/▼ or j/k",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Scroll | "),
        Span::styled(
            "c/m/p/n",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Sort | "),
        Span::styled(
            "/",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Filter | "),
        Span::styled(
            "P-Core: 0-3 (Lion Cove) | LP-E: 4-7 (Skymont)",
            Style::default().fg(Color::Yellow),
        ),
    ]);

    let footer = Paragraph::new(footer_text);
    frame.render_widget(footer, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::HardwareCollector;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_ui_renders_mock_snapshot_without_panic() {
        let mut collector = HardwareCollector::new_mock();
        let snapshot = collector.collect_snapshot(
            Some("test_marker".to_string()),
            &["llama".to_string()],
            true,
        );
        let ui_state = UiState::default();

        let backend = TestBackend::new(140, 45);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_ui(f, &snapshot, &ui_state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut content = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                content.push_str(buffer[(x, y)].symbol());
            }
            content.push('\n');
        }

        assert!(content.contains("MUTOP-AI"));
        assert!(content.contains("CPU Subsystem"));
        assert!(content.contains("LionCove"));
        assert!(content.contains("Skymont"));
        assert!(content.contains("Dual-Accelerator"));
        assert!(content.contains("Arc 140V"));
        assert!(content.contains("AI Boost"));
        assert!(content.contains("LPDDR5X RAM"));
        assert!(content.contains("Package Pwr"));
        assert!(content.contains("Process List"));
    }

    #[test]
    fn test_ui_state_defaults() {
        let state = UiState::default();
        assert_eq!(state.process_sort, ProcessSortMode::CpuDesc);
        assert!(state.process_filter.is_empty());
        assert!(!state.filter_mode);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn test_format_process_table_title() {
        let mut state = UiState::default();
        assert_eq!(format_process_table_title(&state, 20), " Process List ");

        state.scroll_offset = 5;
        assert_eq!(
            format_process_table_title(&state, 20),
            " Process List [Offset: +5/20] "
        );

        state.process_filter = "llama".to_string();
        assert_eq!(
            format_process_table_title(&state, 2),
            " Process List [Filter: llama] [Offset: +5/2] "
        );

        state.scroll_offset = 0;
        assert_eq!(
            format_process_table_title(&state, 2),
            " Process List [Filter: llama] "
        );

        state.filter_mode = true;
        assert_eq!(
            format_process_table_title(&state, 2),
            " Process List [Filter: llama_] "
        );
    }
}
