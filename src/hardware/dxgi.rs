// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

#[cfg(windows)]
pub mod win_dxgi {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
    pub struct Luid {
        pub low_part: u32,
        pub high_part: i32,
    }

    impl Luid {
        pub fn to_u64(&self) -> u64 {
            ((self.high_part as u64) << 32) | (self.low_part as u64)
        }

        pub fn pdh_pattern(&self) -> String {
            format!(
                "luid_0x{:08x}_0x{:08x}",
                self.high_part as u32, self.low_part
            )
        }

        pub fn from_u64(val: u64) -> Self {
            Self {
                low_part: (val & 0xFFFF_FFFF) as u32,
                high_part: ((val >> 32) & 0xFFFF_FFFF) as i32,
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct AdapterInfo {
        pub description: String,
        pub vendor_id: u32,
        pub device_id: u32,
        pub luid: Luid,
        pub shared_sys_mem_mb: u64,
        pub dedicated_video_mem_mb: u64,
        pub is_npu: bool,
        pub is_gpu: bool,
    }

    // DXCore GUIDs and COM vtables
    // IID_IDXCoreAdapterFactory = {78ee5945-c36e-4b13-a669-005dd11c0f06}
    const IID_IDXCORE_ADAPTER_FACTORY: [u8; 16] = [
        0x45, 0x59, 0xee, 0x78, 0x6e, 0xc3, 0x13, 0x4b, 0xa6, 0x69, 0x00, 0x5d, 0xd1, 0x1c, 0x0f,
        0x06,
    ];

    // IID_IDXCoreAdapterList = {526c7776-40e9-459b-b711-f32ad76dfc28}
    const IID_IDXCORE_ADAPTER_LIST: [u8; 16] = [
        0x76, 0x77, 0x6c, 0x52, 0xe9, 0x40, 0x9b, 0x45, 0xb7, 0x11, 0xf3, 0x2a, 0xd7, 0x6d, 0xfc,
        0x28,
    ];

    // IID_IDXCoreAdapter = {f0db4c7f-cf5a-42a2-a815-239442e5d070}
    const IID_IDXCORE_ADAPTER: [u8; 16] = [
        0x7f, 0x4c, 0xdb, 0xf0, 0x5a, 0xcf, 0xa2, 0x42, 0xa8, 0x15, 0x23, 0x94, 0x42, 0xe5, 0xd0,
        0x70,
    ];

    // DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU = {d46140c4-add7-451b-9e56-06fe8c3b58ed}
    const DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU: [u8; 16] = [
        0xc4, 0x40, 0x61, 0xd4, 0xd7, 0xad, 0x1b, 0x45, 0x9e, 0x56, 0x06, 0xfe, 0x8c, 0x3b, 0x58,
        0xed,
    ];

    // DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE = {248e2800-a793-4724-abaa-23a6de1be090}
    const DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE: [u8; 16] = [
        0x00, 0x28, 0x8e, 0x24, 0x93, 0xa7, 0x24, 0x47, 0xab, 0xaa, 0x23, 0xa6, 0xde, 0x1b, 0xe0,
        0x90,
    ];

    // DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML = {b71b0d41-1088-422f-a27c-0250b7d3a988}
    const DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML: [u8; 16] = [
        0x41, 0x0d, 0x1b, 0xb7, 0x88, 0x10, 0x2f, 0x42, 0xa2, 0x7c, 0x02, 0x50, 0xb7, 0xd3, 0xa9,
        0x88,
    ];

    #[repr(C)]
    struct DxcoreHardwareId {
        vendor_id: u32,
        device_id: u32,
        sub_sys_id: u32,
        revision: u32,
    }

    type DxcoreCreateAdapterFactoryFn =
        unsafe extern "system" fn(riid: *const [u8; 16], pp_factory: *mut *mut c_void) -> i32;

    pub fn enumerate_dxcore_npu_adapters() -> Vec<AdapterInfo> {
        let mut results = Vec::new();
        unsafe {
            let dxcore = match libloading::Library::new("dxcore.dll") {
                Ok(lib) => lib,
                Err(_) => return results,
            };

            let create_factory: Result<libloading::Symbol<DxcoreCreateAdapterFactoryFn>, _> =
                dxcore.get(b"DXCoreCreateAdapterFactory\0");
            let create_factory = match create_factory {
                Ok(sym) => sym,
                Err(_) => return results,
            };

            let mut factory: *mut c_void = std::ptr::null_mut();
            if create_factory(&IID_IDXCORE_ADAPTER_FACTORY, &mut factory) != 0 || factory.is_null()
            {
                return results;
            }

            let vtable = *(factory as *const *const usize);
            // CreateAdapterList is index 3
            let create_adapter_list_ptr = *vtable.add(3);
            let create_adapter_list: unsafe extern "system" fn(
                this: *mut c_void,
                num_attributes: u32,
                filter_attributes: *const [u8; 16],
                riid: *const [u8; 16],
                ppv_adapter_list: *mut *mut c_void,
            ) -> i32 = std::mem::transmute(create_adapter_list_ptr);

            let release_factory_ptr = *vtable.add(2);
            let release_factory: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                std::mem::transmute(release_factory_ptr);

            // Query single attributes independently to avoid logical AND filtering
            let filter_attrs = [
                DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU,
                DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE,
                DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML,
            ];

            for attr in &filter_attrs {
                let mut list: *mut c_void = std::ptr::null_mut();
                if create_adapter_list(factory, 1, attr, &IID_IDXCORE_ADAPTER_LIST, &mut list) == 0
                    && !list.is_null()
                {
                    let list_vtable = *(list as *const *const usize);
                    let get_adapter_count_ptr = *list_vtable.add(4);
                    let get_adapter_count: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                        std::mem::transmute(get_adapter_count_ptr);

                    let get_adapter_ptr = *list_vtable.add(3);
                    let get_adapter: unsafe extern "system" fn(
                        this: *mut c_void,
                        index: u32,
                        riid: *const [u8; 16],
                        ppv_adapter: *mut *mut c_void,
                    ) -> i32 = std::mem::transmute(get_adapter_ptr);

                    let release_list_ptr = *list_vtable.add(2);
                    let release_list: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                        std::mem::transmute(release_list_ptr);

                    let count = get_adapter_count(list);
                    for i in 0..count {
                        let mut adapter: *mut c_void = std::ptr::null_mut();
                        if get_adapter(list, i, &IID_IDXCORE_ADAPTER, &mut adapter) == 0
                            && !adapter.is_null()
                        {
                            let adapter_vtable = *(adapter as *const *const usize);
                            let get_property_ptr = *adapter_vtable.add(6);
                            let get_property: unsafe extern "system" fn(
                                this: *mut c_void,
                                property: u32,
                                buffer_size: usize,
                                property_data: *mut c_void,
                            )
                                -> i32 = std::mem::transmute(get_property_ptr);

                            let get_property_size_ptr = *adapter_vtable.add(7);
                            let get_property_size: unsafe extern "system" fn(
                                this: *mut c_void,
                                property: u32,
                                buffer_size: *mut usize,
                            )
                                -> i32 = std::mem::transmute(get_property_size_ptr);

                            let release_adapter_ptr = *adapter_vtable.add(2);
                            let release_adapter: unsafe extern "system" fn(
                                this: *mut c_void,
                            )
                                -> u32 = std::mem::transmute(release_adapter_ptr);

                            // Property 0: InstanceLuid
                            let mut luid = Luid::default();
                            let _ = get_property(
                                adapter,
                                0, // InstanceLuid
                                std::mem::size_of::<Luid>(),
                                &mut luid as *mut _ as *mut c_void,
                            );

                            // Property 2: DriverDescription (null-terminated c_char)
                            let mut desc_size: usize = 0;
                            let mut description = String::new();
                            if get_property_size(adapter, 2, &mut desc_size) == 0 && desc_size > 0 {
                                let mut desc_buf = vec![0u8; desc_size];
                                if get_property(
                                    adapter,
                                    2, // DriverDescription
                                    desc_size,
                                    desc_buf.as_mut_ptr() as *mut c_void,
                                ) == 0
                                {
                                    let desc_len =
                                        desc_buf.iter().position(|&c| c == 0).unwrap_or(desc_size);
                                    description =
                                        String::from_utf8_lossy(&desc_buf[..desc_len]).to_string();
                                }
                            }

                            // Property 3: HardwareID
                            let mut hwid: DxcoreHardwareId = std::mem::zeroed();
                            let _ = get_property(
                                adapter,
                                3, // HardwareID
                                std::mem::size_of::<DxcoreHardwareId>(),
                                &mut hwid as *mut _ as *mut c_void,
                            );

                            // Property 9: SharedSystemMemory (u64 bytes)
                            let mut shared_bytes: u64 = 0;
                            let _ = get_property(
                                adapter,
                                9, // SharedSystemMemory
                                std::mem::size_of::<u64>(),
                                &mut shared_bytes as *mut _ as *mut c_void,
                            );

                            let desc_lower = description.to_lowercase();
                            let is_intel_npu = hwid.vendor_id == 0x8086 && hwid.device_id == 0x643E;
                            let is_npu = is_intel_npu
                                || desc_lower.contains("ai boost")
                                || desc_lower.contains("npu");

                            if is_intel_npu && description.is_empty() {
                                description = "Intel(R) AI Boost".to_string();
                            }

                            if is_npu && !results.iter().any(|r| r.luid.to_u64() == luid.to_u64()) {
                                results.push(AdapterInfo {
                                    description,
                                    vendor_id: hwid.vendor_id,
                                    device_id: hwid.device_id,
                                    luid,
                                    shared_sys_mem_mb: shared_bytes / (1024 * 1024),
                                    dedicated_video_mem_mb: 0,
                                    is_npu: true,
                                    is_gpu: false,
                                });
                            }

                            release_adapter(adapter);
                        }
                    }
                    release_list(list);
                }
            }
            release_factory(factory);
        }

        results
    }

    #[repr(C)]
    struct DxgiAdapterDesc1 {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        sub_sys_id: u32,
        revision: u32,
        dedicated_video_memory: usize,
        dedicated_system_memory: usize,
        shared_system_memory: usize,
        adapter_luid: Luid,
        flags: u32,
    }

    type CreateDxgiFactory1Fn =
        unsafe extern "system" fn(riid: *const [u8; 16], pp_factory: *mut *mut c_void) -> i32;

    const IID_IDXGI_FACTORY1: [u8; 16] = [
        0x78, 0xae, 0x0a, 0x77, 0x6f, 0xf2, 0xba, 0x4d, 0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3,
        0x87,
    ];

    pub fn enumerate_dxgi_adapters() -> Vec<AdapterInfo> {
        let mut results = Vec::new();
        unsafe {
            let dxgi = match libloading::Library::new("dxgi.dll") {
                Ok(lib) => lib,
                Err(_) => return results,
            };

            let create_factory: Result<libloading::Symbol<CreateDxgiFactory1Fn>, _> =
                dxgi.get(b"CreateDXGIFactory1\0");
            let create_factory = match create_factory {
                Ok(sym) => sym,
                Err(_) => return results,
            };

            let mut factory: *mut c_void = std::ptr::null_mut();
            if create_factory(&IID_IDXGI_FACTORY1, &mut factory) != 0 || factory.is_null() {
                return results;
            }

            let vtable = *(factory as *const *const usize);
            let enum_adapters1_ptr = *vtable.add(12);
            let enum_adapters1: unsafe extern "system" fn(
                this: *mut c_void,
                adapter_idx: u32,
                pp_adapter: *mut *mut c_void,
            ) -> i32 = std::mem::transmute(enum_adapters1_ptr);

            let release_factory_ptr = *vtable.add(2);
            let release_factory: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                std::mem::transmute(release_factory_ptr);

            let mut idx = 0u32;
            loop {
                let mut adapter: *mut c_void = std::ptr::null_mut();
                if enum_adapters1(factory, idx, &mut adapter) != 0 || adapter.is_null() {
                    break;
                }
                idx += 1;

                let adapter_vtable = *(adapter as *const *const usize);
                let get_desc1_ptr = *adapter_vtable.add(10);
                let get_desc1: unsafe extern "system" fn(
                    this: *mut c_void,
                    p_desc: *mut DxgiAdapterDesc1,
                ) -> i32 = std::mem::transmute(get_desc1_ptr);

                let release_adapter_ptr = *adapter_vtable.add(2);
                let release_adapter: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                    std::mem::transmute(release_adapter_ptr);

                let mut desc: DxgiAdapterDesc1 = std::mem::zeroed();
                if get_desc1(adapter, &mut desc) == 0 && (desc.flags & 2) == 0 {
                    let desc_len = desc.description.iter().position(|&c| c == 0).unwrap_or(128);
                    let name = String::from_utf16_lossy(&desc.description[..desc_len]);
                    let name_lower = name.to_lowercase();

                    let is_npu = desc.device_id == 0x643E
                        || name_lower.contains("ai boost")
                        || name_lower.contains("npu");

                    let is_gpu = !is_npu
                        && (desc.device_id == 0x64A0
                            || desc.device_id == 0x6420
                            || name_lower.contains("arc")
                            || name_lower.contains("graphics"));

                    results.push(AdapterInfo {
                        description: name,
                        vendor_id: desc.vendor_id,
                        device_id: desc.device_id,
                        luid: desc.adapter_luid,
                        shared_sys_mem_mb: desc.shared_system_memory as u64 / (1024 * 1024),
                        dedicated_video_mem_mb: desc.dedicated_video_memory as u64 / (1024 * 1024),
                        is_npu,
                        is_gpu,
                    });
                }

                release_adapter(adapter);
            }

            release_factory(factory);
        }

        results
    }

    pub fn enumerate_all_adapters() -> Vec<AdapterInfo> {
        let mut adapters = enumerate_dxgi_adapters();
        let npu_adapters = enumerate_dxcore_npu_adapters();
        let gpu_uma_budget = adapters
            .iter()
            .find(|a| a.is_gpu)
            .map(|a| a.shared_sys_mem_mb)
            .unwrap_or(0);

        for mut npu in npu_adapters {
            if npu.shared_sys_mem_mb == 0 && gpu_uma_budget > 0 {
                npu.shared_sys_mem_mb = gpu_uma_budget;
            }
            if !adapters
                .iter()
                .any(|a| a.luid.to_u64() == npu.luid.to_u64())
            {
                adapters.push(npu);
            }
        }
        adapters
    }
}

#[cfg(not(windows))]
pub mod not_win_dxgi {
    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
    pub struct Luid {
        pub low_part: u32,
        pub high_part: i32,
    }

    impl Luid {
        pub fn to_u64(&self) -> u64 {
            ((self.high_part as u64) << 32) | (self.low_part as u64)
        }

        pub fn pdh_pattern(&self) -> String {
            format!(
                "luid_0x{:08x}_0x{:08x}",
                self.high_part as u32, self.low_part
            )
        }

        pub fn from_u64(val: u64) -> Self {
            Self {
                low_part: (val & 0xFFFF_FFFF) as u32,
                high_part: ((val >> 32) & 0xFFFF_FFFF) as i32,
            }
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct AdapterInfo {
        pub description: String,
        pub vendor_id: u32,
        pub device_id: u32,
        pub luid: Luid,
        pub shared_sys_mem_mb: u64,
        pub dedicated_video_mem_mb: u64,
        pub is_npu: bool,
        pub is_gpu: bool,
    }

    pub fn enumerate_dxgi_adapters() -> Vec<AdapterInfo> {
        Vec::new()
    }

    pub fn enumerate_all_adapters() -> Vec<AdapterInfo> {
        Vec::new()
    }
}

#[cfg(windows)]
pub use win_dxgi::*;

#[cfg(not(windows))]
pub use not_win_dxgi::*;
