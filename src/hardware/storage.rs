// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::hardware::pdh::win_pdh::PdhTelemetryState;
use crate::snapshot::types::{NvmeSmartHealth, NvmeStorageSnapshot};
#[cfg(windows)]
use std::time::Duration;
use std::time::Instant;

#[derive(Default)]
pub struct StorageCollectorState {
    pub last_slow_poll: Option<Instant>,
    pub cached_capacity_free: (f64, f64, f32),
    pub cached_smart: Option<NvmeSmartHealth>,
    pub cached_pcie: (u8, u8, u8, u8, bool),
}

pub fn collect_storage_snapshot(
    state: &mut StorageCollectorState,
    pdh_state: Option<&PdhTelemetryState>,
    mock: bool,
) -> NvmeStorageSnapshot {
    if mock {
        return mock_storage_snapshot();
    }
    #[cfg(windows)]
    {
        collect_windows_storage(state, pdh_state)
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        let _ = pdh_state;
        collect_linux_storage()
    }
}

fn mock_storage_snapshot() -> NvmeStorageSnapshot {
    let capacity_gb = 512.0;
    let free_gb = 340.0;
    let used_pct = (((capacity_gb - free_gb) / capacity_gb) * 100.0) as f32;

    NvmeStorageSnapshot {
        capacity_gb,
        free_gb,
        used_pct,
        current_gen: 4,
        current_lanes: 4,
        max_gen: 4,
        max_lanes: 4,
        is_link_degraded: false,
        read_mb_s: 120.5,
        write_mb_s: 45.2,
        read_iops: 2400.0,
        write_iops: 850.0,
        queue_depth: 1.4,
        avg_read_latency_ms: 0.12,
        avg_write_latency_ms: 0.18,
        busy_pct: 14.5,
        smart_health: Some(NvmeSmartHealth {
            composite_temp_c: 41.0,
            controller_temp_c: Some(44.0),
            nand_temp_c: Some(39.0),
            available_spare_pct: 100,
            percentage_used_wear: 1,
            data_read_gb: 4250,
            data_written_gb: 3120,
            unsafe_shutdowns: 4,
            warning_throttle_minutes: 0,
            critical_throttle_minutes: 0,
        }),
    }
}

#[cfg(windows)]
fn collect_windows_storage(
    state: &mut StorageCollectorState,
    pdh_state: Option<&PdhTelemetryState>,
) -> NvmeStorageSnapshot {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let should_poll_slow = state
        .last_slow_poll
        .map(|t| t.elapsed() >= Duration::from_secs(2))
        .unwrap_or(true);

    if should_poll_slow {
        let mut free_bytes_avail: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free_bytes: u64 = 0;

        let success = unsafe {
            let path: Vec<u16> = "C:\\\0".encode_utf16().collect();
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut free_bytes_avail,
                &mut total_bytes,
                &mut total_free_bytes,
            )
        };

        if success != 0 && total_bytes > 0 {
            let cap = total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let free = total_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let used = (((cap - free) / cap) * 100.0) as f32;
            state.cached_capacity_free = (cap, free, used);
        }

        state.cached_smart = try_query_nvme_smart_health();
        state.cached_pcie = query_pcie_link_status();
        state.last_slow_poll = Some(Instant::now());
    }

    let (capacity_gb, free_gb, used_pct) = state.cached_capacity_free;
    let smart_health = state.cached_smart.clone();
    let (current_gen, current_lanes, max_gen, max_lanes, is_link_degraded) = state.cached_pcie;

    let mut read_mb_s = 0.0;
    let mut write_mb_s = 0.0;
    let mut read_iops = 0.0;
    let mut write_iops = 0.0;
    let mut queue_depth = 0.0;
    let mut avg_read_latency_ms = 0.0;
    let mut avg_write_latency_ms = 0.0;
    let mut busy_pct = 0.0;

    if let Some(pdh) = pdh_state {
        if let Some(v) = pdh.read_double("disk_read_bytes") {
            read_mb_s = v / (1024.0 * 1024.0);
        }
        if let Some(v) = pdh.read_double("disk_write_bytes") {
            write_mb_s = v / (1024.0 * 1024.0);
        }
        if let Some(v) = pdh.read_double("disk_reads") {
            read_iops = v;
        }
        if let Some(v) = pdh.read_double("disk_writes") {
            write_iops = v;
        }
        if let Some(v) = pdh.read_double("disk_q_len") {
            queue_depth = v;
        }
        if let Some(v) = pdh.read_double("disk_time") {
            busy_pct = (v as f32).clamp(0.0, 100.0);
        }
        if let Some(v) = pdh.read_double("disk_read_lat") {
            avg_read_latency_ms = v * 1000.0;
        }
        if let Some(v) = pdh.read_double("disk_write_lat") {
            avg_write_latency_ms = v * 1000.0;
        }
    }

    NvmeStorageSnapshot {
        capacity_gb,
        free_gb,
        used_pct,
        current_gen,
        current_lanes,
        max_gen,
        max_lanes,
        is_link_degraded,
        read_mb_s,
        write_mb_s,
        read_iops,
        write_iops,
        queue_depth,
        avg_read_latency_ms,
        avg_write_latency_ms,
        busy_pct,
        smart_health,
    }
}

#[cfg(windows)]
fn try_query_nvme_smart_health() -> Option<NvmeSmartHealth> {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let drive_path: Vec<u16> = "\\\\.\\PhysicalDrive0\0".encode_utf16().collect();
    let handle = unsafe {
        CreateFileW(
            drive_path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    #[repr(C)]
    struct StorageProtocolQuery {
        property_id: u32,
        query_type: u32,
        protocol_type: u32,
        data_type: u32,
        protocol_data_request_value: u32,
        protocol_data_request_sub_value: u32,
        protocol_data_offset: u32,
        protocol_data_length: u32,
    }

    let mut query = StorageProtocolQuery {
        property_id: 50,
        query_type: 0,
        protocol_type: 17,
        data_type: 2,
        protocol_data_request_value: 0x02,
        protocol_data_request_sub_value: 0,
        protocol_data_offset: std::mem::size_of::<StorageProtocolQuery>() as u32,
        protocol_data_length: 512,
    };

    let mut out_buf = vec![0u8; 1024];
    let mut bytes_returned: u32 = 0;

    let res = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut query as *mut _ as *const std::ffi::c_void,
            std::mem::size_of::<StorageProtocolQuery>() as u32,
            out_buf.as_mut_ptr() as *mut std::ffi::c_void,
            out_buf.len() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };

    unsafe { CloseHandle(handle) };

    if res != 0 && bytes_returned >= 512 {
        let offset = query.protocol_data_offset as usize;
        if offset + 512 <= out_buf.len() {
            let data = &out_buf[offset..];
            let comp_temp_raw = u16::from_le_bytes([data[1], data[2]]);
            let composite_temp_c = if comp_temp_raw > 273 {
                (comp_temp_raw - 273) as f32
            } else {
                0.0
            };
            let available_spare_pct = data[3];
            let percentage_used_wear = data[5];
            let unsafe_shutdowns = u64::from_le_bytes([
                data[112], data[113], data[114], data[115], data[116], data[117], data[118],
                data[119],
            ]);
            let warning_throttle_minutes =
                u32::from_le_bytes([data[176], data[177], data[178], data[179]]);
            let critical_throttle_minutes =
                u32::from_le_bytes([data[180], data[181], data[182], data[183]]);

            return Some(NvmeSmartHealth {
                composite_temp_c,
                controller_temp_c: None,
                nand_temp_c: None,
                available_spare_pct,
                percentage_used_wear,
                data_read_gb: 0,
                data_written_gb: 0,
                unsafe_shutdowns,
                warning_throttle_minutes,
                critical_throttle_minutes,
            });
        }
    }

    None
}

#[cfg(windows)]
fn query_pcie_link_status() -> (u8, u8, u8, u8, bool) {
    (0, 0, 0, 0, false)
}

#[cfg(not(windows))]
fn collect_linux_storage() -> NvmeStorageSnapshot {
    mock_storage_snapshot()
}
