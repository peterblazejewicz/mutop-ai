// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::snapshot::types::{
    LoopbackAiSnapshot, NetworkInterfaceSnapshot, NetworkLoopbackSnapshot, Wifi7Snapshot,
};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Default)]
pub struct NetworkCollectorState {
    pub prev_octets: HashMap<u32, (u64, u64, Instant)>,
}

pub fn collect_network_snapshot(
    state: &mut NetworkCollectorState,
    mock: bool,
) -> NetworkLoopbackSnapshot {
    if mock {
        return mock_network_snapshot();
    }

    #[cfg(windows)]
    {
        collect_windows_network(state)
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        collect_linux_network()
    }
}

fn mock_network_snapshot() -> NetworkLoopbackSnapshot {
    NetworkLoopbackSnapshot {
        interfaces: vec![
            NetworkInterfaceSnapshot {
                name: "Intel(R) Ethernet Controller I226-V".to_string(),
                rx_mbps: 18.4,
                tx_mbps: 4.2,
                link_speed_mbps: 2500,
                oper_status: "Up".to_string(),
                in_errors: 0,
                out_discards: 0,
            },
            NetworkInterfaceSnapshot {
                name: "Intel(R) Wi-Fi 7 BE201 320MHz".to_string(),
                rx_mbps: 45.0,
                tx_mbps: 12.8,
                link_speed_mbps: 2882,
                oper_status: "Up".to_string(),
                in_errors: 0,
                out_discards: 0,
            },
        ],
        wifi7: Some(Wifi7Snapshot {
            phy_type: "EHT (802.11be)".to_string(),
            rssi_dbm: -52,
            signal_quality_pct: 94,
            rx_rate_mbps: 2882,
            tx_rate_mbps: 2882,
            channel: 36,
            is_mlo: true,
        }),
        loopback: LoopbackAiSnapshot {
            llama_server_listening: true,
            llama_server_port: 8080,
            mcp_gateway_listening: true,
            mcp_gateway_port: 50051,
            active_loopback_connections: 3,
            loopback_bytes_in: Some(10485760),
            loopback_bytes_out: Some(5242880),
        },
    }
}

#[cfg(windows)]
fn collect_windows_network(state: &mut NetworkCollectorState) -> NetworkLoopbackSnapshot {
    let interfaces = query_windows_interfaces(state);
    let wifi7 = query_windows_wifi7();
    let loopback = query_windows_loopback();

    NetworkLoopbackSnapshot {
        interfaces,
        wifi7,
        loopback,
    }
}

#[cfg(windows)]
fn query_windows_interfaces(state: &mut NetworkCollectorState) -> Vec<NetworkInterfaceSnapshot> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };

    let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    let status = unsafe { GetIfTable2(&mut table_ptr) };

    let mut results = Vec::new();
    let now = Instant::now();

    if status == 0 && !table_ptr.is_null() {
        unsafe {
            let table = &*table_ptr;
            let rows = std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize);

            for row in rows {
                // Filter for physical Ethernet (6) or Wi-Fi (71)
                if row.Type == 6 || row.Type == 71 {
                    let desc_len = row.Description.iter().position(|&c| c == 0).unwrap_or(256);
                    let name = String::from_utf16_lossy(&row.Description[..desc_len]);

                    let oper_status = if row.OperStatus == 1 {
                        "Up".to_string()
                    } else {
                        "Down".to_string()
                    };

                    let curr_in = row.InOctets;
                    let curr_out = row.OutOctets;

                    let (rx_mbps, tx_mbps) = if let Some(&(prev_in, prev_out, prev_time)) =
                        state.prev_octets.get(&row.InterfaceIndex)
                    {
                        let dt_secs = now.duration_since(prev_time).as_secs_f64();
                        if dt_secs > 0.0 {
                            let in_bits = (curr_in.saturating_sub(prev_in) as f64) * 8.0;
                            let out_bits = (curr_out.saturating_sub(prev_out) as f64) * 8.0;
                            (
                                in_bits / (dt_secs * 1_000_000.0),
                                out_bits / (dt_secs * 1_000_000.0),
                            )
                        } else {
                            (0.0, 0.0)
                        }
                    } else {
                        (0.0, 0.0)
                    };

                    state
                        .prev_octets
                        .insert(row.InterfaceIndex, (curr_in, curr_out, now));

                    results.push(NetworkInterfaceSnapshot {
                        name,
                        rx_mbps,
                        tx_mbps,
                        link_speed_mbps: row.TransmitLinkSpeed / 1_000_000,
                        oper_status,
                        in_errors: row.InErrors,
                        out_discards: row.OutDiscards,
                    });
                }
            }

            FreeMibTable(table_ptr as *const std::ffi::c_void);
        }
    }

    results
}

#[cfg(windows)]
fn query_windows_wifi7() -> Option<Wifi7Snapshot> {
    unsafe {
        let wlan = libloading::Library::new("wlanapi.dll").ok()?;

        type WlanOpenHandleFn = unsafe extern "system" fn(
            dw_client_version: u32,
            p_reserved: *const std::ffi::c_void,
            pdw_negotiated_version: *mut u32,
            ph_client_handle: *mut *mut std::ffi::c_void,
        ) -> u32;

        type WlanCloseHandleFn = unsafe extern "system" fn(
            h_client_handle: *mut std::ffi::c_void,
            p_reserved: *const std::ffi::c_void,
        ) -> u32;

        type WlanEnumInterfacesFn = unsafe extern "system" fn(
            h_client_handle: *mut std::ffi::c_void,
            p_reserved: *const std::ffi::c_void,
            pp_interface_list: *mut *mut WlanInterfaceInfoList,
        ) -> u32;

        type WlanQueryInterfaceFn = unsafe extern "system" fn(
            h_client_handle: *mut std::ffi::c_void,
            p_interface_guid: *const [u8; 16],
            op_code: u32,
            p_reserved: *const std::ffi::c_void,
            pdw_data_size: *mut u32,
            pp_data: *mut *mut std::ffi::c_void,
            p_wlan_opcode_value_type: *mut u32,
        ) -> u32;

        type WlanFreeMemoryFn = unsafe extern "system" fn(p_memory: *mut std::ffi::c_void);

        #[repr(C)]
        struct WlanInterfaceInfo {
            interface_guid: [u8; 16],
            interface_description: [u16; 256],
            is_state: u32,
        }

        #[repr(C)]
        struct WlanInterfaceInfoList {
            number_of_items: u32,
            index: u32,
            interface_info: [WlanInterfaceInfo; 1],
        }

        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct WlanRealtimeConnectionQuality {
            dot11_phy_type: u32,
            link_quality: u32,
            rx_rate: u32,
            tx_rate: u32,
            is_mlo_connection: u32,
        }

        let open_handle: libloading::Symbol<WlanOpenHandleFn> =
            wlan.get(b"WlanOpenHandle\0").ok()?;
        let close_handle: libloading::Symbol<WlanCloseHandleFn> =
            wlan.get(b"WlanCloseHandle\0").ok()?;
        let enum_interfaces: libloading::Symbol<WlanEnumInterfacesFn> =
            wlan.get(b"WlanEnumInterfaces\0").ok()?;
        let query_interface: libloading::Symbol<WlanQueryInterfaceFn> =
            wlan.get(b"WlanQueryInterface\0").ok()?;
        let free_memory: libloading::Symbol<WlanFreeMemoryFn> =
            wlan.get(b"WlanFreeMemory\0").ok()?;

        let mut client_version = 0u32;
        let mut handle: *mut std::ffi::c_void = std::ptr::null_mut();
        if open_handle(2, std::ptr::null(), &mut client_version, &mut handle) != 0
            || handle.is_null()
        {
            return None;
        }

        let mut list_ptr: *mut WlanInterfaceInfoList = std::ptr::null_mut();
        if enum_interfaces(handle, std::ptr::null(), &mut list_ptr) != 0 || list_ptr.is_null() {
            close_handle(handle, std::ptr::null());
            return None;
        }

        let count = (*list_ptr).number_of_items;
        if count == 0 {
            free_memory(list_ptr as *mut _);
            close_handle(handle, std::ptr::null());
            return None;
        }

        let first_guid = (*list_ptr).interface_info[0].interface_guid;
        free_memory(list_ptr as *mut _);

        let mut rssi_data_size = 0u32;
        let mut rssi_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut opcode_type = 0u32;
        let mut rssi_dbm = -60;

        if query_interface(
            handle,
            &first_guid,
            0x10000102,
            std::ptr::null(),
            &mut rssi_data_size,
            &mut rssi_ptr,
            &mut opcode_type,
        ) == 0
            && !rssi_ptr.is_null()
        {
            rssi_dbm = *(rssi_ptr as *const i32);
            free_memory(rssi_ptr);
        }

        let mut qual_data_size = 0u32;
        let mut qual_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut quality = WlanRealtimeConnectionQuality::default();

        if query_interface(
            handle,
            &first_guid,
            19,
            std::ptr::null(),
            &mut qual_data_size,
            &mut qual_ptr,
            &mut opcode_type,
        ) == 0
            && !qual_ptr.is_null()
        {
            quality = *(qual_ptr as *const WlanRealtimeConnectionQuality);
            free_memory(qual_ptr);
        }

        close_handle(handle, std::ptr::null());

        let is_eht = quality.dot11_phy_type == 10;
        let phy_type = if is_eht {
            "EHT (802.11be)".to_string()
        } else {
            "HE (802.11ax)".to_string()
        };

        Some(Wifi7Snapshot {
            phy_type,
            rssi_dbm,
            signal_quality_pct: quality.link_quality.min(100) as u8,
            rx_rate_mbps: quality.rx_rate / 1000,
            tx_rate_mbps: quality.tx_rate / 1000,
            channel: 36,
            is_mlo: quality.is_mlo_connection != 0,
        })
    }
}

#[cfg(windows)]
fn query_windows_loopback() -> LoopbackAiSnapshot {
    use windows_sys::Win32::NetworkManagement::IpHelper::GetExtendedTcpTable;

    #[repr(C)]
    struct MibTcpRowOwnerPid {
        dw_state: u32,
        dw_local_addr: u32,
        dw_local_port: u32,
        dw_remote_addr: u32,
        dw_remote_port: u32,
        dw_owning_pid: u32,
    }

    let mut llama_listening = false;
    let mut mcp_listening = false;
    let mut active_conns = 0usize;

    let mut buf_size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut buf_size,
            0,
            2, // AF_INET
            5, // TCP_TABLE_OWNER_PID_ALL
            0,
        );
    }

    if buf_size > 0 {
        let mut buf = vec![0u8; buf_size as usize];
        let res = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                &mut buf_size,
                0,
                2,
                5,
                0,
            )
        };

        if res == 0 {
            let num_entries = unsafe { *(buf.as_ptr() as *const u32) };
            let rows_ptr = unsafe { buf.as_ptr().add(4) as *const MibTcpRowOwnerPid };

            for i in 0..num_entries as usize {
                let row = unsafe { &*rows_ptr.add(i) };
                let local_port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                let remote_port = u16::from_be((row.dw_remote_port & 0xFFFF) as u16);

                let is_loopback = row.dw_local_addr == 0x0100007F || row.dw_local_addr == 0;

                if is_loopback {
                    if row.dw_state == 2 {
                        if local_port == 8080 {
                            llama_listening = true;
                        } else if local_port == 50051 || local_port == 8081 {
                            mcp_listening = true;
                        }
                    } else if row.dw_state == 5
                        && (local_port == 8080
                            || local_port == 50051
                            || remote_port == 8080
                            || remote_port == 50051)
                    {
                        active_conns += 1;
                    }
                }
            }
        }
    }

    LoopbackAiSnapshot {
        llama_server_listening: llama_listening,
        llama_server_port: 8080,
        mcp_gateway_listening: mcp_listening,
        mcp_gateway_port: 50051,
        active_loopback_connections: active_conns,
        loopback_bytes_in: None,
        loopback_bytes_out: None,
    }
}

#[cfg(not(windows))]
fn collect_linux_network() -> NetworkLoopbackSnapshot {
    mock_network_snapshot()
}
