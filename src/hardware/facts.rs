// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

use crate::snapshot::types::{HostInfo, PrivilegeTier};

pub fn get_host_info(mock: bool) -> HostInfo {
    if mock {
        return HostInfo {
            board: "LattePanda Mu Ultra (DFR1295 + DFR1293)".to_string(),
            soc: "Intel Core Ultra 7 256V (Lunar Lake)".to_string(),
            hostname: "lattepanda-mu".to_string(),
            os_build: "Windows 11 Pro 24H2 (10.0.26100.9711)".to_string(),
            architecture: "x86_64".to_string(),
            privilege_tier: PrivilegeTier::Tier1Unprivileged,
            total_ai_tops: 115,
            npu_tops: 47,
            gpu_tops: 64,
            cpu_tops: 5,
        };
    }

    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    #[cfg(windows)]
    let (os_build, privilege_tier) = {
        let os_str = detect_windows_build();
        let tier = detect_windows_privilege();
        (os_str, tier)
    };

    #[cfg(not(windows))]
    let (os_build, privilege_tier) = {
        let os_str = std::fs::read_to_string("/proc/version")
            .map(|s| s.lines().next().unwrap_or("Linux").to_string())
            .unwrap_or_else(|_| "Linux".to_string());
        let is_root = unsafe { libc::geteuid() == 0 };
        let tier = if is_root {
            PrivilegeTier::Tier2ElevatedAdmin
        } else {
            PrivilegeTier::Tier1Unprivileged
        };
        (os_str, tier)
    };

    HostInfo {
        board: "LattePanda Mu Ultra (DFR1295 + DFR1293)".to_string(),
        soc: "Intel Core Ultra 7 256V (Lunar Lake)".to_string(),
        hostname,
        os_build,
        architecture: std::env::consts::ARCH.to_string(),
        privilege_tier,
        total_ai_tops: 115,
        npu_tops: 47,
        gpu_tops: 64,
        cpu_tops: 5,
    }
}

#[cfg(windows)]
fn detect_windows_build() -> String {
    #[repr(C)]
    struct OsVersionInfoExW {
        os_version_info_size: u32,
        major_version: u32,
        minor_version: u32,
        build_number: u32,
        platform_id: u32,
        csd_version: [u16; 128],
        service_pack_major: u16,
        service_pack_minor: u16,
        suite_mask: u16,
        product_type: u8,
        reserved: u8,
    }

    extern "system" {
        fn RtlGetVersion(version_info: *mut OsVersionInfoExW) -> i32;
    }

    let mut info: OsVersionInfoExW = unsafe { std::mem::zeroed() };
    info.os_version_info_size = std::mem::size_of::<OsVersionInfoExW>() as u32;
    let status = unsafe { RtlGetVersion(&mut info) };
    if status == 0 {
        format!(
            "Windows 11 Build {}.{}.{}",
            info.major_version, info.minor_version, info.build_number
        )
    } else {
        "Windows 11".to_string()
    }
}

#[cfg(windows)]
fn detect_windows_privilege() -> PrivilegeTier {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } != 0 {
        let mut elevation: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
        let mut return_len: u32 = 0;
        let success = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut return_len,
            )
        };
        unsafe { CloseHandle(token) };
        if success != 0 && elevation.TokenIsElevated != 0 {
            return PrivilegeTier::Tier2ElevatedAdmin;
        }
    }
    PrivilegeTier::Tier1Unprivileged
}
