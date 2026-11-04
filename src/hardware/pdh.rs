// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

#[cfg(windows)]
pub mod win_pdh {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    pub const PDH_FMT_DOUBLE: u32 = 0x00000200;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct PDH_FMT_COUNTERVALUE {
        pub c_status: u32,
        pub double_value: f64,
    }

    #[repr(C)]
    pub struct PDH_FMT_COUNTERVALUE_ITEM_W {
        pub sz_name: *mut u16,
        pub fmt_value: PDH_FMT_COUNTERVALUE,
    }

    #[link(name = "pdh")]
    extern "system" {
        pub fn PdhOpenQueryW(
            sz_data_source: *const u16,
            dw_user_data: usize,
            ph_query: *mut usize,
        ) -> i32;

        pub fn PdhAddEnglishCounterW(
            h_query: usize,
            sz_full_counter_path: *const u16,
            dw_user_data: usize,
            ph_counter: *mut usize,
        ) -> i32;

        pub fn PdhCollectQueryData(h_query: usize) -> i32;

        pub fn PdhGetFormattedCounterValue(
            h_counter: usize,
            dw_format: u32,
            lp_type: *mut u32,
            p_value: *mut PDH_FMT_COUNTERVALUE,
        ) -> i32;

        pub fn PdhGetFormattedCounterArrayW(
            h_counter: usize,
            dw_format: u32,
            lpdw_buffer_size: *mut u32,
            lpdw_item_count: *mut u32,
            item_buffer: *mut PDH_FMT_COUNTERVALUE_ITEM_W,
        ) -> i32;

        pub fn PdhCloseQuery(h_query: usize) -> i32;
    }

    pub struct PdhTelemetryState {
        query: usize,
        counters: HashMap<String, usize>,
        last_collect: Mutex<Option<Instant>>,
    }

    impl PdhTelemetryState {
        pub fn new() -> Option<Self> {
            let mut q: usize = 0;
            let status = unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) };
            if status != 0 || q == 0 {
                return None;
            }

            let mut state = Self {
                query: q,
                counters: HashMap::new(),
                last_collect: Mutex::new(None),
            };

            // Register all standard counters
            state.add_counter("gpu_engines", "\\GPU Engine(*)\\Utilization Percentage");
            state.add_counter("gpu_proc_mem", "\\GPU Process Memory(*)\\Shared Usage");
            state.add_counter("gpu_adapter_mem", "\\GPU Adapter Memory(*)\\Shared Usage");
            state.add_counter("rapl_pkg", "\\Energy Meter(RAPL_Package0_PKG)\\Power");
            state.add_counter("rapl_pp0", "\\Energy Meter(RAPL_Package0_PP0)\\Power");
            state.add_counter("rapl_pp1", "\\Energy Meter(RAPL_Package0_PP1)\\Power");
            state.add_counter("rapl_dram", "\\Energy Meter(RAPL_Package0_DRAM)\\Power");
            state.add_counter(
                "thermals",
                "\\Thermal Zone Information(*)\\High Precision Temperature",
            );
            state.add_counter(
                "disk_read_bytes",
                "\\PhysicalDisk(_Total)\\Disk Read Bytes/sec",
            );
            state.add_counter(
                "disk_write_bytes",
                "\\PhysicalDisk(_Total)\\Disk Write Bytes/sec",
            );
            state.add_counter("disk_reads", "\\PhysicalDisk(_Total)\\Disk Reads/sec");
            state.add_counter("disk_writes", "\\PhysicalDisk(_Total)\\Disk Writes/sec");
            state.add_counter(
                "disk_q_len",
                "\\PhysicalDisk(_Total)\\Current Disk Queue Length",
            );
            state.add_counter("disk_time", "\\PhysicalDisk(_Total)\\% Disk Time");
            state.add_counter(
                "disk_read_lat",
                "\\PhysicalDisk(_Total)\\Avg. Disk sec/Read",
            );
            state.add_counter(
                "disk_write_lat",
                "\\PhysicalDisk(_Total)\\Avg. Disk sec/Write",
            );

            // Prime the initial query sample so subsequent tick has a valid rate delta!
            unsafe {
                PdhCollectQueryData(state.query);
            }
            if let Ok(mut guard) = state.last_collect.lock() {
                *guard = Some(Instant::now());
            }

            Some(state)
        }

        fn add_counter(&mut self, key: &str, path: &str) -> bool {
            let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut h_counter: usize = 0;
            let status =
                unsafe { PdhAddEnglishCounterW(self.query, path_w.as_ptr(), 0, &mut h_counter) };
            if status == 0 && h_counter != 0 {
                self.counters.insert(key.to_string(), h_counter);
                true
            } else {
                false
            }
        }

        pub fn collect(&self) -> bool {
            if let Ok(guard) = self.last_collect.lock() {
                if let Some(last) = *guard {
                    let elapsed = last.elapsed();
                    if elapsed < Duration::from_millis(40) {
                        std::thread::sleep(Duration::from_millis(40) - elapsed);
                    }
                }
            }
            let res = unsafe { PdhCollectQueryData(self.query) == 0 };
            if let Ok(mut guard) = self.last_collect.lock() {
                *guard = Some(Instant::now());
            }
            res
        }

        pub fn read_double(&self, key: &str) -> Option<f64> {
            let h_counter = *self.counters.get(key)?;
            let mut val = PDH_FMT_COUNTERVALUE::default();
            let mut val_type: u32 = 0;
            let status = unsafe {
                PdhGetFormattedCounterValue(h_counter, PDH_FMT_DOUBLE, &mut val_type, &mut val)
            };
            if status == 0 && (val.c_status == 0 || val.c_status == 1) {
                Some(val.double_value)
            } else {
                None
            }
        }

        pub fn read_array(&self, key: &str) -> Vec<(String, f64)> {
            let mut results = Vec::new();
            let Some(&h_counter) = self.counters.get(key) else {
                return results;
            };

            let mut buffer_size: u32 = 0;
            let mut item_count: u32 = 0;

            unsafe {
                PdhGetFormattedCounterArrayW(
                    h_counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    std::ptr::null_mut(),
                );
            }

            if buffer_size == 0 {
                return results;
            }

            let mut buffer = vec![0u8; buffer_size as usize];
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    h_counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W,
                )
            };

            if status == 0 && item_count > 0 {
                let items = buffer.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W;
                for i in 0..item_count as usize {
                    let item = unsafe { &*items.add(i) };
                    if !item.sz_name.is_null()
                        && (item.fmt_value.c_status == 0 || item.fmt_value.c_status == 1)
                    {
                        let name = unsafe { wide_ptr_to_string(item.sz_name) };
                        results.push((name, item.fmt_value.double_value));
                    }
                }
            }

            results
        }
    }

    impl Drop for PdhTelemetryState {
        fn drop(&mut self) {
            if self.query != 0 {
                unsafe { PdhCloseQuery(self.query) };
            }
        }
    }

    unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}

#[cfg(not(windows))]
pub mod win_pdh {
    #[derive(Debug, Default)]
    pub struct PdhTelemetryState;

    impl PdhTelemetryState {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn collect(&self) -> bool {
            false
        }
        pub fn read_double(&self, _key: &str) -> Option<f64> {
            None
        }
        pub fn read_array(&self, _key: &str) -> Vec<(String, f64)> {
            Vec::new()
        }
    }
}
