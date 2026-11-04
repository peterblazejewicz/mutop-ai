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
    pub pcie_attempts: u8,
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
        if state.pcie_attempts < 3 {
            let pcie = query_pcie_link_status();
            if pcie.0 > 0 || pcie.1 > 0 {
                state.cached_pcie = pcie;
                state.pcie_attempts = 3;
            } else {
                state.pcie_attempts += 1;
            }
        }
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

    // PropertyId = 52: StorageDeviceTemperatureProperty (standard stornvme.sys)
    let temp_query = [52u32, 0u32, 0u32];
    let mut temp_out = [0u8; 128];
    let mut temp_bytes_ret = 0u32;
    let temp_res = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            temp_query.as_ptr() as *const std::ffi::c_void,
            std::mem::size_of_val(&temp_query) as u32,
            temp_out.as_mut_ptr() as *mut std::ffi::c_void,
            temp_out.len() as u32,
            &mut temp_bytes_ret,
            std::ptr::null_mut(),
        )
    };

    unsafe { CloseHandle(handle) };

    let mut p52_composite: Option<f32> = None;
    let mut p52_controller: Option<f32> = None;
    let mut p52_nand: Option<f32> = None;

    if temp_res != 0 && temp_bytes_ret >= 24 {
        let count = u16::from_le_bytes([temp_out[12], temp_out[13]]) as usize;
        for i in 0..count {
            let offset = 24 + i * 16;
            if offset + 4 <= temp_bytes_ret as usize {
                let idx = u16::from_le_bytes([temp_out[offset], temp_out[offset + 1]]);
                let temp = i16::from_le_bytes([temp_out[offset + 2], temp_out[offset + 3]]);
                if temp > 0 && temp < 150 {
                    let temp_f = temp as f32;
                    match idx {
                        0 => p52_composite = Some(temp_f),
                        1 => p52_nand = Some(temp_f),
                        2 => p52_controller = Some(temp_f),
                        _ => {
                            if p52_composite.is_none() {
                                p52_composite = Some(temp_f);
                            }
                        }
                    }
                }
            }
        }
    }

    if res != 0 && bytes_returned >= 512 {
        let offset = query.protocol_data_offset as usize;
        if offset + 512 <= out_buf.len() {
            let data = &out_buf[offset..];
            let comp_temp_raw = u16::from_le_bytes([data[1], data[2]]);
            let mut composite_temp_c = if comp_temp_raw > 273 {
                (comp_temp_raw - 273) as f32
            } else {
                0.0
            };
            if composite_temp_c == 0.0 {
                if let Some(t) = p52_composite {
                    composite_temp_c = t;
                }
            }
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
                controller_temp_c: p52_controller,
                nand_temp_c: p52_nand,
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

    if p52_composite.is_some() || p52_controller.is_some() {
        return Some(NvmeSmartHealth {
            composite_temp_c: p52_composite.unwrap_or(0.0),
            controller_temp_c: p52_controller,
            nand_temp_c: p52_nand,
            available_spare_pct: 100,
            percentage_used_wear: 0,
            data_read_gb: 0,
            data_written_gb: 0,
            unsafe_shutdowns: 0,
            warning_throttle_minutes: 0,
            critical_throttle_minutes: 0,
        });
    }

    None
}

#[cfg(windows)]
#[repr(C)]
struct DevPropKey {
    fmtid: [u8; 16],
    pid: u32,
}

#[cfg(windows)]
type CmGetDeviceIdListSizeWFn =
    unsafe extern "system" fn(pul_len: *mut u32, psz_filter: *const u16, ul_flags: u32) -> u32;
#[cfg(windows)]
type CmGetDeviceIdListWFn = unsafe extern "system" fn(
    psz_filter: *const u16,
    buffer: *mut u16,
    buffer_len: u32,
    ul_flags: u32,
) -> u32;
#[cfg(windows)]
type CmLocateDevNodeWFn = unsafe extern "system" fn(
    pdn_dev_inst: *mut u32,
    p_device_id: *const u16,
    ul_flags: u32,
) -> u32;
#[cfg(windows)]
type CmGetParentFn =
    unsafe extern "system" fn(pdn_dev_inst: *mut u32, dn_dev_inst: u32, ul_flags: u32) -> u32;
#[cfg(windows)]
type CmGetDevNodePropertyWFn = unsafe extern "system" fn(
    dn_dev_inst: u32,
    property_key: *const DevPropKey,
    property_type: *mut u32,
    property_buffer: *mut u8,
    property_buffer_size: *mut u32,
    ul_flags: u32,
) -> u32;

#[cfg(windows)]
struct CfgMgr32 {
    _lib: libloading::Library,
    cm_get_size: CmGetDeviceIdListSizeWFn,
    cm_get_list: CmGetDeviceIdListWFn,
    cm_locate: CmLocateDevNodeWFn,
    cm_get_parent: CmGetParentFn,
    cm_get_prop: CmGetDevNodePropertyWFn,
}

#[cfg(windows)]
static CFGMGR32: std::sync::OnceLock<Option<CfgMgr32>> = std::sync::OnceLock::new();

#[cfg(windows)]
fn get_cfgmgr32() -> Option<&'static CfgMgr32> {
    CFGMGR32
        .get_or_init(|| unsafe {
            let lib = libloading::Library::new("cfgmgr32.dll").ok()?;
            let cm_get_size = *lib
                .get::<CmGetDeviceIdListSizeWFn>(b"CM_Get_Device_ID_List_SizeW\0")
                .ok()?;
            let cm_get_list = *lib
                .get::<CmGetDeviceIdListWFn>(b"CM_Get_Device_ID_ListW\0")
                .ok()?;
            let cm_locate = *lib
                .get::<CmLocateDevNodeWFn>(b"CM_Locate_DevNodeW\0")
                .ok()?;
            let cm_get_parent = *lib.get::<CmGetParentFn>(b"CM_Get_Parent\0").ok()?;
            let cm_get_prop = *lib
                .get::<CmGetDevNodePropertyWFn>(b"CM_Get_DevNode_PropertyW\0")
                .ok()?;
            Some(CfgMgr32 {
                _lib: lib,
                cm_get_size,
                cm_get_list,
                cm_locate,
                cm_get_parent,
                cm_get_prop,
            })
        })
        .as_ref()
}

#[cfg(windows)]
fn query_pcie_link_status() -> (u8, u8, u8, u8, bool) {
    let cm = match get_cfgmgr32() {
        Some(c) => c,
        None => return (0, 0, 0, 0, false),
    };

    let pci_filter: Vec<u16> = "PCI\0".encode_utf16().collect();
    let mut len: u32 = 0;
    let cr = unsafe { (cm.cm_get_size)(&mut len, pci_filter.as_ptr(), 0) };
    if cr != 0 || len == 0 {
        return (0, 0, 0, 0, false);
    }

    let mut buf: Vec<u16> = vec![0; len as usize];
    let cr = unsafe { (cm.cm_get_list)(pci_filter.as_ptr(), buf.as_mut_ptr(), len, 0) };
    if cr != 0 {
        return (0, 0, 0, 0, false);
    }

    let mut dev_ids = Vec::new();
    let mut cur = Vec::new();
    for &ch in &buf {
        if ch == 0 {
            if !cur.is_empty() {
                dev_ids.push(String::from_utf16_lossy(&cur));
                cur.clear();
            }
        } else {
            cur.push(ch);
        }
    }

    // DEVPKEY_Device_Service: {a45c254e-df1c-4efd-8020-67d146a850e0}, pid 6
    let service_guid = [
        0x4e, 0x25, 0x5c, 0xa4, 0x1c, 0xdf, 0xfd, 0x4e, 0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50,
        0xe0,
    ];

    let mut target_dev_inst: Option<u32> = None;

    for id in &dev_ids {
        let id_u = id.to_uppercase();
        if !id_u.starts_with("PCI\\") {
            continue;
        }

        let dev_id_w: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        let mut dev_inst: u32 = 0;
        let cr = unsafe { (cm.cm_locate)(&mut dev_inst, dev_id_w.as_ptr(), 0) };
        if cr != 0 {
            continue;
        }

        // Query DEVPKEY_Device_Service
        let key = DevPropKey {
            fmtid: service_guid,
            pid: 6,
        };
        let mut ptype = 0u32;
        let mut val_buf = [0u8; 256];
        let mut vlen = val_buf.len() as u32;
        let prop_cr = unsafe {
            (cm.cm_get_prop)(
                dev_inst,
                &key,
                &mut ptype,
                val_buf.as_mut_ptr(),
                &mut vlen,
                0,
            )
        };

        let mut service_name = String::new();
        if prop_cr == 0 && vlen >= 2 {
            let u16_len = (vlen as usize) / 2;
            let u16_slice: &[u16] =
                unsafe { std::slice::from_raw_parts(val_buf.as_ptr() as *const u16, u16_len) };
            let term_len = u16_slice.iter().position(|&c| c == 0).unwrap_or(u16_len);
            service_name = String::from_utf16_lossy(&u16_slice[..term_len]);
        }

        let is_stornvme = service_name.eq_ignore_ascii_case("stornvme")
            || service_name.to_lowercase().contains("nvme");
        let is_nvme_class = id_u.contains("CC_0108");

        if is_stornvme || is_nvme_class {
            target_dev_inst = Some(dev_inst);
            break;
        }
    }

    let dev_inst = match target_dev_inst {
        Some(inst) => inst,
        None => return (0, 0, 0, 0, false),
    };

    // DEVPKEY_PciDevice_* {3ab22e31-8264-4b4e-9af5-a8d2d8e33e62}
    let pci_guid = [
        0x31, 0x2e, 0xb2, 0x3a, 0x64, 0x82, 0x4e, 0x4b, 0x9a, 0xf5, 0xa8, 0xd2, 0xd8, 0xe3, 0x3e,
        0x62,
    ];

    let query_prop = |inst: u32, pid: u32| -> u32 {
        let key = DevPropKey {
            fmtid: pci_guid,
            pid,
        };
        let mut ptype = 0u32;
        let mut val_buf = [0u8; 8];
        let mut vlen = val_buf.len() as u32;
        let cr =
            unsafe { (cm.cm_get_prop)(inst, &key, &mut ptype, val_buf.as_mut_ptr(), &mut vlen, 0) };
        if cr == 0 && vlen >= 4 {
            u32::from_le_bytes([val_buf[0], val_buf[1], val_buf[2], val_buf[3]])
        } else {
            0
        }
    };

    let cur_speed = query_prop(dev_inst, 9) as u8; // CurrentLinkSpeed
    let cur_width = query_prop(dev_inst, 10) as u8; // CurrentLinkWidth
    let max_speed = query_prop(dev_inst, 11) as u8; // MaxLinkSpeed
    let max_width = query_prop(dev_inst, 12) as u8; // MaxLinkWidth

    // Query parent PCIe Root Port link capabilities
    let mut parent_inst: u32 = 0;
    let parent_cr = unsafe { (cm.cm_get_parent)(&mut parent_inst, dev_inst, 0) };
    let (parent_max_speed, parent_max_width) = if parent_cr == 0 && parent_inst != 0 {
        // Try pid 12 (MaxLinkWidth) or pid 13 (SupportedLinkWidth)
        let mut w = query_prop(parent_inst, 12) as u8;
        if w == 0 {
            w = query_prop(parent_inst, 13) as u8;
        }
        // Try pid 11 (MaxLinkSpeed) or pid 31 (SupportedLinkSpeed)
        let mut s = query_prop(parent_inst, 11) as u8;
        if s == 0 {
            s = query_prop(parent_inst, 31) as u8;
        }
        (s, w)
    } else {
        (0, 0)
    };

    let effective_max_width = if parent_max_width > 0 && max_width > 0 {
        max_width.min(parent_max_width)
    } else if max_width > 0 {
        max_width
    } else {
        parent_max_width
    };

    let effective_max_speed = if parent_max_speed > 0 && max_speed > 0 {
        max_speed.min(parent_max_speed)
    } else if max_speed > 0 {
        max_speed
    } else {
        parent_max_speed
    };

    let is_degraded = cur_width > 0 && effective_max_width > 0 && cur_width < effective_max_width;

    (
        cur_speed,
        cur_width,
        effective_max_speed,
        effective_max_width,
        is_degraded,
    )
}

#[cfg(not(windows))]
fn collect_linux_storage() -> NvmeStorageSnapshot {
    mock_storage_snapshot()
}
