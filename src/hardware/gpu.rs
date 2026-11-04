// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::hardware::dxgi::AdapterInfo;
use crate::hardware::pdh::win_pdh::PdhTelemetryState;
use crate::snapshot::types::{AcceleratorProcess, GpuThrottlingFlags, IntelArcGpuSnapshot};

pub fn collect_gpu_snapshot(
    adapters: &[AdapterInfo],
    engine_items: &[(String, f64)],
    pdh_state: Option<&PdhTelemetryState>,
    mock: bool,
) -> IntelArcGpuSnapshot {
    if mock {
        return mock_gpu_snapshot();
    }
    #[cfg(windows)]
    {
        collect_windows_gpu(adapters, engine_items, pdh_state)
    }
    #[cfg(not(windows))]
    {
        let _ = adapters;
        let _ = engine_items;
        let _ = pdh_state;
        collect_linux_gpu()
    }
}

fn mock_gpu_snapshot() -> IntelArcGpuSnapshot {
    IntelArcGpuSnapshot {
        name: "Intel(R) Arc(TM) 140V GPU".to_string(),
        architecture: "8 Xe2-LPG cores (Battlemage)".to_string(),
        gpu_tops: 64,
        adapter_luid: Some(0x1002),
        total_utilization_pct: 22.5,
        render_3d_pct: 8.0,
        compute_pct: 14.5,
        copy_pct: 2.1,
        video_decode_pct: 0.0,
        clock_mhz: 1850,
        temperature_c: Some(44.0),
        power_w: Some(5.2),
        throttling: GpuThrottlingFlags {
            power_limited: false,
            temp_limited: false,
            current_limited: false,
            voltage_limited: false,
        },
        shared_memory_used_mb: 1024,
        shared_memory_budget_mb: 15360,
        active_processes: vec![AcceleratorProcess {
            pid: 1420,
            name: "llama-server.exe".to_string(),
            engine: "Compute (Vulkan/SYCL)".to_string(),
            shared_memory_mb: 768,
            utilization_pct: 14.5,
        }],
    }
}

#[cfg(windows)]
fn collect_windows_gpu(
    adapters: &[AdapterInfo],
    engine_items: &[(String, f64)],
    pdh_state: Option<&PdhTelemetryState>,
) -> IntelArcGpuSnapshot {
    let gpu_adapter = adapters.iter().find(|a| a.is_gpu);

    let Some(adapter) = gpu_adapter else {
        return IntelArcGpuSnapshot {
            name: "Intel(R) Arc(TM) 140V GPU".to_string(),
            architecture: "8 Xe2-LPG cores (Battlemage)".to_string(),
            gpu_tops: 64,
            adapter_luid: None,
            total_utilization_pct: 0.0,
            render_3d_pct: 0.0,
            compute_pct: 0.0,
            copy_pct: 0.0,
            video_decode_pct: 0.0,
            clock_mhz: 0,
            temperature_c: None,
            power_w: None,
            throttling: GpuThrottlingFlags::default(),
            shared_memory_used_mb: 0,
            shared_memory_budget_mb: 0,
            active_processes: vec![],
        };
    };

    let luid_val = adapter.luid.to_u64();
    let luid_pattern = adapter.luid.pdh_pattern().to_lowercase();

    let mut render_3d_pct = 0.0f32;
    let mut compute_pct = 0.0f32;
    let mut copy_pct = 0.0f32;
    let mut video_decode_pct = 0.0f32;
    let mut shared_memory_used_mb = 0u64;
    let mut active_processes = Vec::new();

    for (name, val) in engine_items {
        let name_lower = name.to_lowercase();
        if name_lower.contains(&luid_pattern) {
            let v = *val as f32;
            if name_lower.contains("engtype_3d") {
                render_3d_pct += v;
            } else if name_lower.contains("engtype_compute") {
                compute_pct += v;
            } else if name_lower.contains("engtype_copy") {
                copy_pct += v;
            } else if name_lower.contains("engtype_videodecode") {
                video_decode_pct += v;
            }

            if let Some(pid) = extract_pid_from_instance(&name_lower) {
                if !active_processes
                    .iter()
                    .any(|p: &AcceleratorProcess| p.pid == pid)
                {
                    active_processes.push(AcceleratorProcess {
                        pid,
                        name: format!("PID {}", pid),
                        engine: "GPU Engine".to_string(),
                        shared_memory_mb: 0,
                        utilization_pct: v,
                    });
                }
            }
        }
    }

    if let Some(pdh) = pdh_state {
        let mem_items = pdh.read_array("gpu_proc_mem");
        for (name, bytes) in mem_items {
            let name_lower = name.to_lowercase();
            if name_lower.contains(&luid_pattern) {
                let mb = (bytes / (1024.0 * 1024.0)) as u64;
                shared_memory_used_mb += mb;
                if let Some(pid) = extract_pid_from_instance(&name_lower) {
                    if let Some(proc) = active_processes.iter_mut().find(|p| p.pid == pid) {
                        proc.shared_memory_mb = mb;
                    }
                }
            }
        }
    }

    let total_utilization_pct = render_3d_pct + compute_pct + copy_pct + video_decode_pct;
    let igcl_telemetry = try_read_igcl();

    let (clock_mhz, temperature_c, power_w, throttling) = match igcl_telemetry {
        Some(t) => (t.clock_mhz, t.temp_c, t.power_w, t.throttling),
        None => (0, None, None, GpuThrottlingFlags::default()),
    };

    IntelArcGpuSnapshot {
        name: adapter.description.clone(),
        architecture: "8 Xe2-LPG cores (Battlemage)".to_string(),
        gpu_tops: 64,
        adapter_luid: Some(luid_val),
        total_utilization_pct,
        render_3d_pct,
        compute_pct,
        copy_pct,
        video_decode_pct,
        clock_mhz,
        temperature_c,
        power_w,
        throttling,
        shared_memory_used_mb,
        shared_memory_budget_mb: adapter.shared_sys_mem_mb,
        active_processes,
    }
}

#[cfg(windows)]
fn extract_pid_from_instance(name: &str) -> Option<u32> {
    if let Some(pos) = name.find("pid_") {
        let after = &name[pos + 4..];
        let end = after.find('_').unwrap_or(after.len());
        after[..end].parse::<u32>().ok()
    } else {
        None
    }
}

#[cfg(windows)]
struct IgclTelemetry {
    clock_mhz: u32,
    temp_c: Option<f32>,
    power_w: Option<f32>,
    throttling: GpuThrottlingFlags,
}

#[cfg(windows)]
#[repr(C)]
struct CtlInitArgs {
    size: u32,
    version: u8,
    zero_version: u8,
    flags: u32,
    supported_version: u32,
    app_version: *const std::ffi::c_char,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct CtlPowerTelemetry {
    size: u32,
    version: u8,
    supported: u8,
    timestamp: u64,
    gpu_energy_counter: f64,
    gpu_voltage: f64,
    gpu_current_limited: u8,
    gpu_temperature_limited: u8,
    gpu_power_limited: u8,
    gpu_voltage_limited: u8,
    gpu_current_temperature: f64,
    gpu_current_clock_frequency: f64,
}

#[cfg(windows)]
fn try_read_igcl() -> Option<IgclTelemetry> {
    type CtlInitFn =
        unsafe extern "C" fn(args: *mut CtlInitArgs, handle: *mut *mut std::ffi::c_void) -> i32;
    type CtlCloseFn = unsafe extern "C" fn(handle: *mut std::ffi::c_void) -> i32;
    type CtlEnumDevicesFn = unsafe extern "C" fn(
        handle: *mut std::ffi::c_void,
        count: *mut u32,
        devices: *mut *mut std::ffi::c_void,
    ) -> i32;
    type CtlPowerTelemetryGetFn =
        unsafe extern "C" fn(device: *mut std::ffi::c_void, args: *mut CtlPowerTelemetry) -> i32;

    struct IgclLib {
        _lib: libloading::Library,
        ctl_init: CtlInitFn,
        ctl_close: CtlCloseFn,
        ctl_enum: CtlEnumDevicesFn,
        ctl_telemetry: CtlPowerTelemetryGetFn,
    }

    static IGCL: std::sync::OnceLock<Option<IgclLib>> = std::sync::OnceLock::new();

    let igcl = IGCL.get_or_init(|| unsafe {
        let lib = libloading::Library::new("ControlLib.dll").ok()?;
        let ctl_init = *lib.get::<CtlInitFn>(b"ctlInit\0").ok()?;
        let ctl_close = *lib.get::<CtlCloseFn>(b"ctlClose\0").ok()?;
        let ctl_enum = *lib.get::<CtlEnumDevicesFn>(b"ctlEnumerateDevices\0").ok()?;
        let ctl_telemetry = *lib
            .get::<CtlPowerTelemetryGetFn>(b"ctlPowerTelemetryGet\0")
            .ok()?;
        Some(IgclLib {
            _lib: lib,
            ctl_init,
            ctl_close,
            ctl_enum,
            ctl_telemetry,
        })
    });

    let igcl = igcl.as_ref()?;

    unsafe {
        let mut args: CtlInitArgs = std::mem::zeroed();
        args.size = std::mem::size_of::<CtlInitArgs>() as u32;
        args.supported_version = 1;

        let mut api_handle: *mut std::ffi::c_void = std::ptr::null_mut();
        if (igcl.ctl_init)(&mut args, &mut api_handle) != 0 || api_handle.is_null() {
            return None;
        }

        let mut count = 0u32;
        if (igcl.ctl_enum)(api_handle, &mut count, std::ptr::null_mut()) != 0 || count == 0 {
            (igcl.ctl_close)(api_handle);
            return None;
        }

        let mut device_handles: Vec<*mut std::ffi::c_void> =
            vec![std::ptr::null_mut(); count as usize];
        if (igcl.ctl_enum)(api_handle, &mut count, device_handles.as_mut_ptr()) != 0 {
            (igcl.ctl_close)(api_handle);
            return None;
        }

        let mut telem: CtlPowerTelemetry = std::mem::zeroed();
        telem.size = std::mem::size_of::<CtlPowerTelemetry>() as u32;

        let status = (igcl.ctl_telemetry)(device_handles[0], &mut telem);
        (igcl.ctl_close)(api_handle);

        if status == 0 {
            let clock_mhz = telem.gpu_current_clock_frequency as u32;
            let temp_c = if telem.gpu_current_temperature > 0.0 {
                Some(telem.gpu_current_temperature as f32)
            } else {
                None
            };
            let throttling = GpuThrottlingFlags {
                power_limited: telem.gpu_power_limited != 0,
                temp_limited: telem.gpu_temperature_limited != 0,
                current_limited: telem.gpu_current_limited != 0,
                voltage_limited: telem.gpu_voltage_limited != 0,
            };

            Some(IgclTelemetry {
                clock_mhz,
                temp_c,
                power_w: None,
                throttling,
            })
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn collect_linux_gpu() -> IntelArcGpuSnapshot {
    mock_gpu_snapshot()
}
