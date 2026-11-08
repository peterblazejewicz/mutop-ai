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

/// Resolves GPU power in Watts from either direct IGCL telemetry or RAPL PP1 energy counter.
///
/// On Intel Lunar Lake Xe2-LPG iGPU, IGCL may not report direct GPU telemetry power,
/// and the Windows PDH RAPL PP1 counter (`\Energy Meter(RAPL_Package0_PP1)\Power`) returns valid
/// 0.0 mW readings when the GPU is parked in RC6 idle sleep state.
pub fn resolve_gpu_power_w(igcl_power_w: Option<f32>, rapl_pp1_mw: Option<f64>) -> Option<f32> {
    if let Some(w) = igcl_power_w {
        Some(w)
    } else if let Some(mw) = rapl_pp1_mw {
        if mw >= 0.0 && mw.is_finite() {
            Some((mw / 1000.0) as f32)
        } else {
            None
        }
    } else {
        None
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
    let gpu_adapters: Vec<&AdapterInfo> = adapters.iter().filter(|a| a.is_gpu).collect();
    let adapter = gpu_adapters
        .iter()
        .find(|a| {
            let pat = a.luid.pdh_pattern().to_lowercase();
            engine_items
                .iter()
                .any(|(name, _)| name.to_lowercase().contains(&pat))
                || pdh_state
                    .map(|p| {
                        p.read_array("gpu_adapter_mem")
                            .iter()
                            .any(|(name, _)| name.to_lowercase().contains(&pat))
                    })
                    .unwrap_or(false)
        })
        .copied()
        .or_else(|| gpu_adapters.first().copied());

    let Some(adapter) = adapter else {
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
        let adapter_mem = pdh.read_array("gpu_adapter_mem");
        for (name, bytes) in &adapter_mem {
            let name_lower = name.to_lowercase();
            if name_lower.contains(&luid_pattern) {
                let mb = (bytes / (1024.0 * 1024.0)) as u64;
                if mb > 0 {
                    shared_memory_used_mb = mb;
                }
            }
        }

        let mem_items = pdh.read_array("gpu_proc_mem");
        let mut proc_sum_mb = 0u64;
        for (name, bytes) in mem_items {
            let name_lower = name.to_lowercase();
            if name_lower.contains(&luid_pattern) {
                let mb = (bytes / (1024.0 * 1024.0)) as u64;
                proc_sum_mb += mb;
                if let Some(pid) = extract_pid_from_instance(&name_lower) {
                    if let Some(proc) = active_processes.iter_mut().find(|p| p.pid == pid) {
                        proc.shared_memory_mb = mb;
                    }
                }
            }
        }
        if shared_memory_used_mb == 0 {
            shared_memory_used_mb = proc_sum_mb;
        }
    }

    let total_utilization_pct = render_3d_pct + compute_pct + copy_pct + video_decode_pct;
    let igcl_telemetry = try_read_igcl();
    let gpu_active = total_utilization_pct >= 5.0;
    let pdh_pp1_mw = pdh_state.and_then(|p| p.read_double("rapl_pp1"));

    let (clock_mhz, temperature_c, power_w, throttling) = match igcl_telemetry {
        Some(t) => (
            t.clock_mhz,
            t.temp_c,
            resolve_gpu_power_w(t.power_w, pdh_pp1_mw),
            GpuThrottlingFlags {
                // On Lunar Lake Xe2-LPG, IGCL asserts power/current/voltage limit bits
                // when the GPU is parked in RC6 idle state; only report non-thermal
                // limiters when the GPU engines are actively utilized.
                power_limited: gpu_active && t.throttling.power_limited,
                temp_limited: t.throttling.temp_limited,
                current_limited: gpu_active && t.throttling.current_limited,
                voltage_limited: gpu_active && t.throttling.voltage_limited,
            },
        ),
        None => (
            0,
            None,
            resolve_gpu_power_w(None, pdh_pp1_mw),
            GpuThrottlingFlags::default(),
        ),
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
    reserved: [u8; 3],
    app_version: u32,
    flags: u32,
    supported_version: u32,
    application_uid: [u8; 16],
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlOcTelemetryItem {
    b_supported: u8,
    reserved1: [u8; 3],
    units: u32,
    value_type: u32,
    reserved2: u32,
    value_f64: f64,
}

#[cfg(windows)]
#[repr(C)]
struct CtlPowerTelemetry {
    size: u32,
    version: u8,
    reserved: [u8; 3],
    time_stamp: CtlOcTelemetryItem,
    gpu_energy_counter: CtlOcTelemetryItem,
    gpu_voltage: CtlOcTelemetryItem,
    gpu_current_clock_frequency: CtlOcTelemetryItem,
    gpu_current_temperature: CtlOcTelemetryItem,
    global_activity_counter: CtlOcTelemetryItem,
    render_compute_activity_counter: CtlOcTelemetryItem,
    media_activity_counter: CtlOcTelemetryItem,
    gpu_power_limited: u8,
    gpu_temperature_limited: u8,
    gpu_current_limited: u8,
    gpu_voltage_limited: u8,
    gpu_utilization_limited: u8,
    reserved_flags: [u8; 3],
    payload_tail: [u8; 232],
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
        let mut args = CtlInitArgs {
            size: std::mem::size_of::<CtlInitArgs>() as u32,
            version: 0,
            reserved: [0; 3],
            app_version: 0x0001_0000,
            flags: 0,
            supported_version: 0,
            application_uid: [0; 16],
        };

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
        telem.version = 0;

        let status = (igcl.ctl_telemetry)(device_handles[0], &mut telem);
        (igcl.ctl_close)(api_handle);

        if status == 0 {
            let clock_mhz = if telem.gpu_current_clock_frequency.b_supported != 0
                && telem.gpu_current_clock_frequency.value_f64 > 0.0
            {
                telem.gpu_current_clock_frequency.value_f64 as u32
            } else {
                0
            };
            let temp_c = if telem.gpu_current_temperature.b_supported != 0
                && telem.gpu_current_temperature.value_f64 > 0.0
            {
                Some(telem.gpu_current_temperature.value_f64 as f32)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_gpu_power_w() {
        // Idle RC6 returns Some(0.0) mW -> must resolve to Some(0.0) W and NOT None
        assert_eq!(resolve_gpu_power_w(None, Some(0.0)), Some(0.0));

        // Active reading in mW -> converted to W (e.g. 1.82 mW -> 0.00182 W)
        let pwr = resolve_gpu_power_w(None, Some(1.82)).expect("expected power");
        assert!((pwr - 0.00182).abs() < 1e-6);

        // PDH counter completely unavailable -> None
        assert_eq!(resolve_gpu_power_w(None, None), None);

        // IGCL telemetry direct power takes precedence even if RAPL is 0.0
        assert_eq!(resolve_gpu_power_w(Some(4.5), Some(0.0)), Some(4.5));

        // Invalid or negative RAPL values return None
        assert_eq!(resolve_gpu_power_w(None, Some(-0.5)), None);
        assert_eq!(resolve_gpu_power_w(None, Some(f64::NAN)), None);
        assert_eq!(resolve_gpu_power_w(None, Some(f64::INFINITY)), None);
        assert_eq!(resolve_gpu_power_w(None, Some(f64::NEG_INFINITY)), None);
    }

    #[cfg(windows)]
    #[test]
    fn test_igcl_struct_sizes_and_offsets() {
        assert_eq!(std::mem::size_of::<CtlInitArgs>(), 36);
        assert_eq!(std::mem::size_of::<CtlOcTelemetryItem>(), 24);
        assert_eq!(std::mem::size_of::<CtlPowerTelemetry>(), 440);

        let telem: CtlPowerTelemetry = unsafe { std::mem::zeroed() };
        let base = &telem as *const _ as usize;
        assert_eq!(&telem.time_stamp as *const _ as usize - base, 8);
        assert_eq!(&telem.gpu_energy_counter as *const _ as usize - base, 32);
        assert_eq!(&telem.gpu_voltage as *const _ as usize - base, 56);
        assert_eq!(
            &telem.gpu_current_clock_frequency as *const _ as usize - base,
            80
        );
        assert_eq!(
            &telem.gpu_current_temperature as *const _ as usize - base,
            104
        );
        assert_eq!(
            &telem.global_activity_counter as *const _ as usize - base,
            128
        );
        assert_eq!(
            &telem.render_compute_activity_counter as *const _ as usize - base,
            152
        );
        assert_eq!(
            &telem.media_activity_counter as *const _ as usize - base,
            176
        );
        assert_eq!(&telem.gpu_power_limited as *const _ as usize - base, 200);
        assert_eq!(
            &telem.gpu_temperature_limited as *const _ as usize - base,
            201
        );
        assert_eq!(&telem.gpu_current_limited as *const _ as usize - base, 202);
        assert_eq!(&telem.gpu_voltage_limited as *const _ as usize - base, 203);
        assert_eq!(
            &telem.gpu_utilization_limited as *const _ as usize - base,
            204
        );
        assert_eq!(&telem.payload_tail as *const _ as usize - base, 208);
    }
}
