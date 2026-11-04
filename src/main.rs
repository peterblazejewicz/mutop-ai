// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};

use edgetop_core::cli::{parse_args, CliAction};
use edgetop_core::{EdgePlatform, TickContext};
use mutop_ai::platform::MutopPlatform;
use mutop_ai::ui::{render_ui, ProcessSortMode, UiState};

fn main() -> Result<()> {
    // Install panic hook restoring terminal raw mode & alternate screen
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let msg = panic_info.to_string();
        if !msg.contains("Broken pipe") {
            default_hook(panic_info);
        }
    }));

    let mut platform = MutopPlatform::new();
    let action = match parse_args(std::env::args(), platform.metadata()) {
        Ok(act) => act,
        Err(err) => {
            eprintln!("{}", err);
            eprintln!("For more information, try '--help'");
            std::process::exit(2);
        }
    };

    match action {
        CliAction::PrintHelp => {
            print!("{}", edgetop_core::cli::format_help(platform.metadata()));
            Ok(())
        }
        CliAction::PrintVersion => {
            println!(
                "{} {}",
                platform.metadata().binary_name,
                platform.metadata().version
            );
            Ok(())
        }
        CliAction::PrintSchema => {
            print!(
                "{}",
                edgetop_core::schema::format_schema_table(
                    platform.metadata(),
                    platform.schema_fields()
                )
            );
            Ok(())
        }
        CliAction::Json(opts) => {
            apply_idle_baseline_options(&mut platform, &opts);
            let ctx = TickContext {
                tracked_patterns: opts.tracked_processes,
                include_threads: opts.include_threads,
                ..Default::default()
            };
            let snap = platform.collect_snapshot(&ctx);
            println!("{}", serde_json::to_string_pretty(&snap)?);
            Ok(())
        }
        CliAction::Oneshot(opts) => {
            apply_idle_baseline_options(&mut platform, &opts);
            let ctx = TickContext {
                tracked_patterns: opts.tracked_processes,
                include_threads: opts.include_threads,
                ..Default::default()
            };
            let snap = platform.collect_snapshot(&ctx);
            print!("{}", platform.format_oneshot(&snap));
            Ok(())
        }
        CliAction::Stream(opts) => {
            apply_idle_baseline_options(&mut platform, &opts);
            let stream_opts = edgetop_core::stream::StreamOptions {
                interval_ms: opts.interval_ms,
                tracked_patterns: opts.tracked_processes,
                include_threads: opts.include_threads,
                duration_secs: opts.duration_secs,
                summary: opts.summary,
                mark_file: opts.mark_file,
                active_threshold_pct: opts.active_threshold_pct,
            };
            let mut stdout = io::stdout();
            let start = Instant::now();
            let accumulator =
                edgetop_core::stream::run_stream(&mut platform, &stream_opts, &mut stdout, || {
                    false
                });
            if opts.summary {
                let elapsed = start.elapsed().as_secs_f64();
                print!(
                    "{}",
                    accumulator.format_report(
                        platform.metadata(),
                        opts.active_threshold_pct,
                        elapsed,
                    )
                );
            }
            Ok(())
        }
        CliAction::McpStdio(opts) => {
            apply_idle_baseline_options(&mut platform, &opts);
            edgetop_core::mcp::run_mcp_stdio(&mut platform, opts.mark_file.as_deref());
            Ok(())
        }
        CliAction::McpHttp(addr, opts) => {
            apply_idle_baseline_options(&mut platform, &opts);
            edgetop_core::mcp::run_mcp_http_server(
                &mut platform,
                &addr,
                opts.interval_ms,
                opts.mark_file.as_deref(),
            )?;
            Ok(())
        }
        CliAction::RunTui(opts) => {
            apply_idle_baseline_options(&mut platform, &opts);
            run_interactive_tui(&mut platform, &opts)
        }
    }
}

fn run_interactive_tui(
    platform: &mut MutopPlatform,
    opts: &edgetop_core::cli::CliOptions,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut ui_state = UiState::default();
    let tick_rate = Duration::from_millis(opts.interval_ms.max(50));
    let mut last_tick = Instant::now();
    let start_time = Instant::now();
    let mut tick_index = 0;

    let tracked_patterns = if opts.tracked_processes.is_empty() {
        vec!["*".to_string()]
    } else {
        opts.tracked_processes.clone()
    };

    let ctx = TickContext {
        interval_ms: Some(opts.interval_ms as f64),
        marker: None,
        events: Vec::new(),
        tracked_patterns: tracked_patterns.clone(),
        include_threads: true,
        tick_index,
        elapsed_run_secs: 0.0,
    };
    let mut current_snapshot = platform.collect_snapshot(&ctx);

    loop {
        terminal.draw(|f| {
            render_ui(f, &current_snapshot, &ui_state);
        })?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Critical Windows 11 ConPTY rule: Filter if key.kind == KeyEventKind::Press
                if key.kind == KeyEventKind::Press {
                    if ui_state.filter_mode {
                        match key.code {
                            KeyCode::Esc | KeyCode::Enter => {
                                ui_state.filter_mode = false;
                            }
                            KeyCode::Backspace => {
                                ui_state.process_filter.pop();
                                ui_state.scroll_offset = 0;
                            }
                            KeyCode::Char(c) => {
                                ui_state.process_filter.push(c);
                                ui_state.scroll_offset = 0;
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                                break;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                ui_state.scroll_offset = ui_state.scroll_offset.saturating_add(1);
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                ui_state.scroll_offset = ui_state.scroll_offset.saturating_sub(1);
                            }
                            KeyCode::PageDown => {
                                ui_state.scroll_offset = ui_state.scroll_offset.saturating_add(10);
                            }
                            KeyCode::PageUp => {
                                ui_state.scroll_offset = ui_state.scroll_offset.saturating_sub(10);
                            }
                            KeyCode::Home | KeyCode::Char('g') => {
                                ui_state.scroll_offset = 0;
                            }
                            KeyCode::Char('c') => {
                                ui_state.process_sort = ProcessSortMode::CpuDesc;
                                ui_state.scroll_offset = 0;
                            }
                            KeyCode::Char('m') => {
                                ui_state.process_sort = ProcessSortMode::MemoryDesc;
                                ui_state.scroll_offset = 0;
                            }
                            KeyCode::Char('p') => {
                                ui_state.process_sort = ProcessSortMode::PidAsc;
                                ui_state.scroll_offset = 0;
                            }
                            KeyCode::Char('n') => {
                                ui_state.process_sort = ProcessSortMode::NameAsc;
                                ui_state.scroll_offset = 0;
                            }
                            KeyCode::Char('/') => {
                                ui_state.filter_mode = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            tick_index += 1;
            let ctx = TickContext {
                interval_ms: Some(last_tick.elapsed().as_secs_f64() * 1000.0),
                marker: None,
                events: Vec::new(),
                tracked_patterns: tracked_patterns.clone(),
                include_threads: true,
                tick_index,
                elapsed_run_secs: start_time.elapsed().as_secs_f64(),
            };
            current_snapshot = platform.collect_snapshot(&ctx);
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn apply_idle_baseline_options(platform: &mut MutopPlatform, opts: &edgetop_core::cli::CliOptions) {
    if opts.calibrate_idle && opts.idle_baseline_w.is_none() {
        if let Some(w) = platform.calibrate_idle_baseline(3, Duration::from_millis(200)) {
            eprintln!("[mutop-ai] Calibrated idle baseline: {:.2} W", w);
        }
    } else if let Some(idle) = opts.idle_baseline_w {
        platform.set_idle_baseline_w(idle);
    }
}
