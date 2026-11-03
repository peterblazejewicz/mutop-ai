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
    DiagnosticStatus, MutopSnapshot, NpuActivityStatus, PrivilegeTier, ProcessAffinityProfile,
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header: Host, SoC, Privilege, Timer Res, Power Mode, Status
            Constraint::Length(8), // CPU Panel (Lion Cove P-cores 0..3 vs Skymont LP-E cores 4..7)
            Constraint::Length(7), // Accelerators: Intel Arc 140V GPU + Intel AI Boost NPU 4
            Constraint::Length(7), // Unified Memory (16GB MoP) + RAPL Power & Thermals
            Constraint::Length(6), // Storage (NVMe Gen4 x4) + Network (2.5GbE, Wi-Fi 7, Loopback)
            Constraint::Min(8),    // Interactive Process & Thread Placement Table
            Constraint::Length(1), // Keybindings & Status Bar
        ])
        .split(size);

    render_header(frame, chunks[0], snapshot, ui_state);
    render_cpu_panel(frame, chunks[1], snapshot);
    render_accelerators_panel(frame, chunks[2], snapshot);
    render_memory_power_panel(frame, chunks[3], snapshot);
    render_storage_network_panel(frame, chunks[4], snapshot);
    render_process_table(frame, chunks[5], snapshot, ui_state);
    render_footer(frame, chunks[6], ui_state);
}

fn make_gauge(pct: f32, width: usize) -> (String, Color) {
    let clamped = pct.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    let bar = format!("[{}{}]", "|".repeat(filled), " ".repeat(empty));
    let color = if clamped >= 85.0 {
        Color::LightRed
    } else if clamped >= 60.0 {
        Color::Yellow
    } else {
        Color::LightGreen
    };
    (bar, color)
}

fn render_header(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot, ui_state: &UiState) {
    let compact = area.width < 120;
    let uptime = snapshot.timing.unbiased_uptime_sec as u64;
    let hours = uptime / 3600;
    let mins = (uptime % 3600) / 60;
    let secs = uptime % 60;

    let tier_str = match snapshot.host.privilege_tier {
        PrivilegeTier::Tier1Unprivileged => "Tier 1 (Unprivileged)",
        PrivilegeTier::Tier2ElevatedAdmin => "Tier 2 (Admin)",
        PrivilegeTier::Tier3SignedDriver => "Tier 3 (Driver)",
    };
    let tier_color = match snapshot.host.privilege_tier {
        PrivilegeTier::Tier1Unprivileged => Color::Cyan,
        PrivilegeTier::Tier2ElevatedAdmin => Color::LightGreen,
        PrivilegeTier::Tier3SignedDriver => Color::Magenta,
    };

    let timer_res = snapshot.timing.timer_resolution.current_ms;
    let timer_res_color = if timer_res <= 1.0 {
        Color::LightGreen
    } else {
        Color::Yellow
    };

    let status_str = match snapshot.ai_diagnostics.status {
        DiagnosticStatus::Healthy => "HEALTHY",
        DiagnosticStatus::Degraded => "DEGRADED",
        DiagnosticStatus::Critical => "CRITICAL",
    };
    let status_color = match snapshot.ai_diagnostics.status {
        DiagnosticStatus::Healthy => Color::LightGreen,
        DiagnosticStatus::Degraded => Color::Yellow,
        DiagnosticStatus::Critical => Color::LightRed,
    };

    let mut line1 = vec![
        Span::styled(
            " MUTOP-AI ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if compact {
                " LP Mu "
            } else {
                " LattePanda Mu Ultra "
            },
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "[Intel Core Ultra 7 256V (Lunar Lake)] ",
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!("[{} AI TOPS] ", snapshot.host.total_ai_tops),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("[{}] ", tier_str), Style::default().fg(tier_color)),
    ];

    if let Some(ref marker) = snapshot.marker {
        line1.push(Span::styled(
            format!("MARK: {} ", marker),
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let line2 = vec![
        Span::styled("Timer: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:.3}ms ", timer_res),
            Style::default().fg(timer_res_color),
        ),
        Span::styled("| Power Mode: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:?} ", snapshot.power_thermals.effective_power_mode),
            Style::default().fg(Color::White),
        ),
        Span::styled("| Uptime: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:02}:{:02}:{:02} ", hours, mins, secs),
            Style::default().fg(Color::White),
        ),
        Span::styled("| AI Health: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            status_str,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        if !snapshot.ai_diagnostics.bottlenecks.is_empty() {
            Span::styled(
                format!(" ({})", snapshot.ai_diagnostics.bottlenecks.join(", ")),
                Style::default().fg(Color::LightRed),
            )
        } else {
            Span::raw("")
        },
    ];

    let paragraph = Paragraph::new(vec![Line::from(line1), Line::from(line2)]).block(
        Block::default()
            .borders(Borders::ALL)
            .title(if ui_state.filter_mode {
                format!(
                    " Filter Active: [{}] (Press Esc to finish) ",
                    ui_state.process_filter
                )
            } else {
                " System Overview ".to_string()
            })
            .style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(paragraph, area);
}

fn render_cpu_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let gauge_w = 12;

    // Left: Lion Cove P-cores 0..3
    let mut p_lines = vec![Line::from(vec![
        Span::styled(
            "Lion Cove P-cores (0..3) Total: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:5.1}%", snapshot.cpu.lion_cove_p_load_pct),
            Style::default().fg(Color::White),
        ),
        if snapshot.cpu.is_throttled {
            Span::styled(
                " [THROTTLED]",
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        },
    ])];

    for i in 0..4 {
        let pct = snapshot.cpu.per_core_pct.get(i).copied().unwrap_or(0.0);
        let mhz = snapshot
            .cpu
            .per_core_current_mhz
            .get(i)
            .copied()
            .unwrap_or(0);
        let max_mhz = snapshot.cpu.per_core_max_mhz.get(i).copied().unwrap_or(0);
        let (gauge, g_col) = make_gauge(pct, gauge_w);

        p_lines.push(Line::from(vec![
            Span::styled(
                format!("  P{} ", i),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:5.1}% ", pct), Style::default().fg(Color::White)),
            Span::styled(gauge, Style::default().fg(g_col)),
            Span::styled(
                format!(" {:4} / {:4} MHz", mhz, max_mhz),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let p_block = Paragraph::new(p_lines).block(
        Block::default()
            .title(" Lion Cove P-Cluster (Performance) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(p_block, cols[0]);

    // Right: Skymont LP-E cores 4..7
    let mut e_lines = vec![Line::from(vec![
        Span::styled(
            "Skymont LP-E cores (4..7) Total: ",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:5.1}%", snapshot.cpu.skymont_lpe_load_pct),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(
                " | Conserved: U:{:.0}% S:{:.0}% Idl:{:.0}%",
                snapshot.cpu.user_pct, snapshot.cpu.system_pct, snapshot.cpu.idle_pct
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ])];

    for i in 4..8 {
        let pct = snapshot.cpu.per_core_pct.get(i).copied().unwrap_or(0.0);
        let mhz = snapshot
            .cpu
            .per_core_current_mhz
            .get(i)
            .copied()
            .unwrap_or(0);
        let max_mhz = snapshot.cpu.per_core_max_mhz.get(i).copied().unwrap_or(0);
        let (gauge, g_col) = make_gauge(pct, gauge_w);

        e_lines.push(Line::from(vec![
            Span::styled(
                format!("  E{} ", i),
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:5.1}% ", pct), Style::default().fg(Color::White)),
            Span::styled(gauge, Style::default().fg(g_col)),
            Span::styled(
                format!(" {:4} / {:4} MHz", mhz, max_mhz),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let e_block = Paragraph::new(e_lines).block(
        Block::default()
            .title(" Skymont LP-E Cluster (Low-Power Efficiency) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::LightGreen)),
    );
    frame.render_widget(e_block, cols[1]);
}

fn render_accelerators_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let gauge_w = 14;

    // Left: Intel Arc 140V GPU (64 TOPS)
    let (gpu_gauge, gpu_col) = make_gauge(snapshot.gpu.total_utilization_pct, gauge_w);
    let gpu_lines = vec![
        Line::from(vec![
            Span::styled("Util: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{:5.1}% ", snapshot.gpu.total_utilization_pct),
                Style::default().fg(Color::White),
            ),
            Span::styled(gpu_gauge, Style::default().fg(gpu_col)),
            Span::styled(
                format!(" | Clock: {} MHz", snapshot.gpu.clock_mhz),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(
                    " | Temp: {}",
                    snapshot
                        .gpu
                        .temperature_c
                        .map(|t| format!("{:.0}°C", t))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("Engines: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("3D: {:.1}% ", snapshot.gpu.render_3d_pct),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Compute: {:.1}% ", snapshot.gpu.compute_pct),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("Copy: {:.1}% ", snapshot.gpu.copy_pct),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("Decode: {:.1}%", snapshot.gpu.video_decode_pct),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Shared RAM: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{} / {} MB",
                    snapshot.gpu.shared_memory_used_mb, snapshot.gpu.shared_memory_budget_mb
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(" | Power: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                snapshot
                    .gpu
                    .power_w
                    .map(|p| format!("{:.1} W", p))
                    .unwrap_or_else(|| "-".to_string()),
                Style::default().fg(Color::LightRed),
            ),
            Span::styled(
                if snapshot.gpu.throttling.power_limited {
                    " [PWR LIMIT]"
                } else {
                    ""
                },
                Style::default().fg(Color::LightRed),
            ),
        ]),
    ];

    let gpu_block = Paragraph::new(gpu_lines).block(
        Block::default()
            .title(format!(" {} (64 TOPS, 8 Xe2-LPG) ", snapshot.gpu.name))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(gpu_block, cols[0]);

    // Right: Intel AI Boost NPU 4 (47 TOPS)
    let (npu_gauge, npu_col) = make_gauge(snapshot.npu.utilization_pct, gauge_w);
    let mut npu_lines = vec![
        Line::from(vec![
            Span::styled("Util: ", Style::default().fg(Color::Magenta)),
            Span::styled(
                format!("{:5.1}% ", snapshot.npu.utilization_pct),
                Style::default().fg(Color::White),
            ),
            Span::styled(npu_gauge, Style::default().fg(npu_col)),
            Span::styled(
                format!(" | Freq: {} MHz", snapshot.npu.current_freq_mhz),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(" | Status: {:?}", snapshot.npu.activity_status),
                Style::default().fg(match snapshot.npu.activity_status {
                    NpuActivityStatus::Idle => Color::DarkGray,
                    _ => Color::LightGreen,
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Shared RAM: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{} / {} MB",
                    snapshot.npu.shared_memory_used_mb, snapshot.npu.shared_memory_budget_mb
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(" | Hardware: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "47 TOPS MCDM Compute Adapter",
                Style::default().fg(Color::Magenta),
            ),
        ]),
    ];

    if let Some(ref slots) = snapshot.npu.slots_metrics {
        npu_lines.push(Line::from(vec![
            Span::styled(
                "llama-server: ",
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Prefill: {:.1} T/s | ", slots.prompt_tokens_per_sec),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Decode: {:.1} T/s | ", slots.generation_tokens_per_sec),
                Style::default().fg(Color::LightGreen),
            ),
            Span::styled(
                format!("Slots: {} / {}", slots.active_slots, slots.total_slots),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    } else {
        npu_lines.push(Line::from(vec![
            Span::styled("llama-server: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if snapshot.network.loopback.llama_server_listening {
                    "Listening on :8080 (Slots Idle)"
                } else {
                    "Offline (no /slots)"
                },
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let npu_block = Paragraph::new(npu_lines).block(
        Block::default()
            .title(format!(" {} (47 TOPS, NPU 4) ", snapshot.npu.name))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Magenta)),
    );
    frame.render_widget(npu_block, cols[1]);
}

fn render_memory_power_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let mop_mb = snapshot.memory.installed_mop_mb;
    let in_use_mb = snapshot.memory.in_use_phys_mb;
    let mem_pct = (in_use_mb as f32 / mop_mb as f32) * 100.0;
    let (mem_gauge, mem_col) = make_gauge(mem_pct, 14);

    let mem_lines = vec![
        Line::from(vec![
            Span::styled("In-Use: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{} / {} MB ({:.1}%) ", in_use_mb, mop_mb, mem_pct),
                Style::default().fg(Color::White),
            ),
            Span::styled(mem_gauge, Style::default().fg(mem_col)),
        ]),
        Line::from(vec![
            Span::styled("Reconciled: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("CPU: {} MB | ", snapshot.memory.pure_cpu_in_use_mb),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Arc Shared: {} MB | ", snapshot.memory.arc_gpu_shared_mb),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("NPU Shared: {} MB", snapshot.memory.npu_shared_mb),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from(vec![
            Span::styled("Aperture / Commit: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("BIOS: {} MB | ", snapshot.memory.bios_stolen_aperture_mb),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!(
                    "Commit: {} / {} MB",
                    snapshot.memory.commit_total_mb, snapshot.memory.commit_limit_mb
                ),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let mem_block = Paragraph::new(mem_lines).block(
        Block::default()
            .title(" Unified Memory (16 GB LPDDR5X-8533 MoP) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(mem_block, cols[0]);

    // Right: RAPL Power & Thermals
    let p_pkg = snapshot.power_thermals.package_w;
    let p_above = snapshot.power_thermals.power_above_idle_w;
    let p_base = snapshot.power_thermals.idle_baseline_w;

    let pwr_lines = vec![
        Line::from(vec![
            Span::styled(
                "Package (PKG): ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:.2} W ", p_pkg),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("(Above idle: +{:.2} W, Baseline: {:.1} W)", p_above, p_base),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Rails: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("PP0 (CPU): {:.2} W | ", snapshot.power_thermals.core_pp0_w),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("PP1 (GPU): {:.2} W | ", snapshot.power_thermals.gpu_pp1_w),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("DRAM: {:.2} W", snapshot.power_thermals.dram_w),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from(vec![
            Span::styled("Thermals: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "ACPI: {} | ",
                    snapshot
                        .power_thermals
                        .acpi_thermal_zone_c
                        .map(|t| format!("{:.0}°C", t))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(
                    "GPU: {} | ",
                    snapshot
                        .power_thermals
                        .gpu_c
                        .map(|t| format!("{:.0}°C", t))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!(
                    "NVMe: {} | ",
                    snapshot
                        .power_thermals
                        .nvme_composite_c
                        .map(|t| format!("{:.0}°C", t))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(
                    "Fan: {}",
                    snapshot
                        .power_thermals
                        .cpu_fan_rpm
                        .map(|r| format!("{} RPM", r))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Style::default().fg(Color::LightGreen),
            ),
        ]),
    ];

    let pwr_block = Paragraph::new(pwr_lines).block(
        Block::default()
            .title(" RAPL Power Rails & Thermals ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(pwr_block, cols[1]);
}

fn render_storage_network_panel(frame: &mut Frame, area: Rect, snapshot: &MutopSnapshot) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let storage_lines = vec![
        Line::from(vec![
            Span::styled(
                format!(
                    "Link: PCIe Gen{} x{} ",
                    snapshot.storage.current_gen, snapshot.storage.current_lanes
                ),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            if snapshot.storage.is_link_degraded {
                Span::styled(
                    "[LINK DEGRADED] ",
                    Style::default()
                        .fg(Color::LightRed)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("[Normal] ", Style::default().fg(Color::LightGreen))
            },
            Span::styled(
                format!(
                    "Capacity: {:.0} / {:.0} GB ({:.1}%)",
                    snapshot.storage.capacity_gb - snapshot.storage.free_gb,
                    snapshot.storage.capacity_gb,
                    snapshot.storage.used_pct
                ),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!(
                    "Read: {:.1} MB/s ({:.0} IOPS) | ",
                    snapshot.storage.read_mb_s, snapshot.storage.read_iops
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(
                    "Write: {:.1} MB/s ({:.0} IOPS) | ",
                    snapshot.storage.write_mb_s, snapshot.storage.write_iops
                ),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Busy: {:.1}%", snapshot.storage.busy_pct),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ];

    let storage_block = Paragraph::new(storage_lines).block(
        Block::default()
            .title(" NVMe Storage (M.2 PCIe Gen4 x4) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::LightGreen)),
    );
    frame.render_widget(storage_block, cols[0]);

    // Right: Network & AI Loopback Sockets
    let wifi_str = if let Some(ref wifi) = snapshot.network.wifi7 {
        format!(
            "({} dBm, {}, MLO: {})",
            wifi.rssi_dbm, wifi.phy_type, wifi.is_mlo
        )
    } else {
        "None".to_string()
    };

    let total_rx_mbps: f64 = snapshot.network.interfaces.iter().map(|i| i.rx_mbps).sum();
    let total_tx_mbps: f64 = snapshot.network.interfaces.iter().map(|i| i.tx_mbps).sum();

    let net_lines = vec![
        Line::from(vec![Span::styled(
            format!(
                "Throughput: Rx: {:.2} Mbps | Tx: {:.2} Mbps",
                total_rx_mbps, total_tx_mbps
            ),
            Style::default().fg(Color::White),
        )]),
        Line::from(vec![
            Span::styled("Wi-Fi 7: ", Style::default().fg(Color::DarkGray)),
            Span::styled(wifi_str, Style::default().fg(Color::LightGreen)),
            Span::styled(" | Loopback: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if snapshot.network.loopback.llama_server_listening {
                    "llama:8080 UP "
                } else {
                    "llama:8080 DOWN "
                },
                Style::default().fg(if snapshot.network.loopback.llama_server_listening {
                    Color::LightGreen
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                if snapshot.network.loopback.mcp_gateway_listening {
                    "MCP Gateway:UP"
                } else {
                    "MCP Gateway:DOWN"
                },
                Style::default().fg(if snapshot.network.loopback.mcp_gateway_listening {
                    Color::LightGreen
                } else {
                    Color::DarkGray
                }),
            ),
        ]),
    ];

    let net_block = Paragraph::new(net_lines).block(
        Block::default()
            .title(" Network (2.5GbE / Wi-Fi 7 BE201) & AI Loopback ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::LightGreen)),
    );
    frame.render_widget(net_block, cols[1]);
}

fn render_process_table(
    frame: &mut Frame,
    area: Rect,
    snapshot: &MutopSnapshot,
    ui_state: &UiState,
) {
    let header_cells = [
        "PID",
        "Name",
        "CPU %",
        "P-Core %",
        "LP-E %",
        "RAM (MB)",
        "GPU (MB)",
        "NPU (MB)",
        "Threads (P/E)",
        "EcoQoS",
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
    let header = Row::new(header_cells).height(1);

    let empty_vec = Vec::new();
    let procs = snapshot.tracked_processes.as_ref().unwrap_or(&empty_vec);

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
            let is_ai = p.name.contains("llama") || p.name.contains("device-commander");
            let name_style = if is_ai {
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            Row::new(vec![
                Span::raw(p.pid.to_string()),
                Span::styled(&p.name, name_style),
                Span::styled(
                    format!("{:5.1}", p.cpu_pct),
                    if p.cpu_pct > 50.0 {
                        Style::default().fg(Color::LightRed)
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

    let sort_label = match ui_state.process_sort {
        ProcessSortMode::CpuDesc => "CPU %",
        ProcessSortMode::MemoryDesc => "Memory",
        ProcessSortMode::PidAsc => "PID",
        ProcessSortMode::NameAsc => "Name",
    };

    let title = format!(
        " Process & Thread Placement Table ({} matching, sorted by {}) ",
        filtered_procs.len(),
        sort_label
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(22),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(15),
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

fn render_footer(frame: &mut Frame, area: Rect, ui_state: &UiState) {
    let text = if ui_state.filter_mode {
        Line::from(vec![
            Span::styled(
                " FILTER MODE: ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" [{}] ", ui_state.process_filter),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                "Type filter text | Press Enter/Esc to confirm | Backspace to erase",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " q/Esc:Quit ",
                Style::default().fg(Color::Black).bg(Color::DarkGray),
            ),
            Span::styled(" c:CPU ", Style::default().fg(Color::Yellow)),
            Span::styled(" m:Mem ", Style::default().fg(Color::Cyan)),
            Span::styled(" p:PID ", Style::default().fg(Color::White)),
            Span::styled(" n:Name ", Style::default().fg(Color::LightGreen)),
            Span::styled(" /:Filter ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                " j/k/Down/Up/PgUp/PgDn:Scroll ",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };

    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, area);
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

        let backend = TestBackend::new(120, 40);
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
        assert!(content.contains("Lion Cove"));
        assert!(content.contains("Skymont"));
        assert!(content.contains("140V"));
        assert!(content.contains("AI Boost"));
        assert!(content.contains("16384 MB"));
    }
}
