// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

//! Optional hardware sensor reader using the Microsoft WHQL-signed PawnIO kernel driver.
//!
//! Provides real CPU package/core DTS temperature via MSR (`IA32_PACKAGE_THERM_STATUS` `0x1B1`)
//! and carrier fan RPM via ITE IT8613E Super I/O on the LattePanda Mu Ultra (`DFR1293`).
//!
//! If PawnIO is not installed (`PawnIOLib.dll` missing or driver absent), this collector
//! gracefully degrades to `None` with zero overhead and zero retries per tick.

#[cfg(windows)]
const INTEL_MSR_BIN: &[u8] = include_bytes!("pawnio/IntelMSR.bin");
#[cfg(windows)]
const LPC_IO_BIN: &[u8] = include_bytes!("pawnio/LpcIO.bin");

#[cfg(windows)]
type PawnioOpenFn = unsafe extern "system" fn(handle: *mut isize) -> i32;
#[cfg(windows)]
type PawnioLoadFn = unsafe extern "system" fn(handle: isize, blob: *const u8, size: usize) -> i32;
#[cfg(windows)]
type PawnioExecuteFn = unsafe extern "system" fn(
    handle: isize,
    name: *const std::ffi::c_char,
    in_buf: *const u64,
    in_size: usize,
    out_buf: *mut u64,
    out_size: usize,
    return_size: *mut usize,
) -> i32;
#[cfg(windows)]
type PawnioCloseFn = unsafe extern "system" fn(handle: isize) -> i32;

#[cfg(windows)]
struct IsaBusMutexGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for IsaBusMutexGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn acquire_isa_bus_mutex() -> Option<IsaBusMutexGuard> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{CreateMutexW, OpenMutexW, WaitForSingleObject};

    const SYNCHRONIZE: u32 = 0x00100000;

    let global_name: Vec<u16> = "Global\\Access_ISABUS.HTP.Method\0"
        .encode_utf16()
        .collect();
    let local_name: Vec<u16> = "Access_ISABUS.HTP.Method\0".encode_utf16().collect();

    let mut h_mutex: HANDLE = unsafe { CreateMutexW(std::ptr::null(), 0, global_name.as_ptr()) };
    if h_mutex.is_null() {
        let err = unsafe { GetLastError() };
        if err == ERROR_ACCESS_DENIED {
            h_mutex = unsafe { OpenMutexW(SYNCHRONIZE, 0, global_name.as_ptr()) };
        }
    }
    if h_mutex.is_null() {
        h_mutex = unsafe { CreateMutexW(std::ptr::null(), 0, local_name.as_ptr()) };
    }
    if h_mutex.is_null() {
        return None;
    }

    let wait_res = unsafe { WaitForSingleObject(h_mutex, 20) };
    if wait_res == WAIT_OBJECT_0 || wait_res == WAIT_ABANDONED {
        Some(IsaBusMutexGuard(h_mutex))
    } else {
        unsafe { CloseHandle(h_mutex) };
        None
    }
}

#[cfg(windows)]
pub struct PawnIoSensorReader {
    _lib: libloading::Library,
    pawnio_execute: PawnioExecuteFn,
    pawnio_close: PawnioCloseFn,
    msr_handle: Option<isize>,
    lpc_handle: Option<isize>,
    tj_max: f32,
    hwm_base: Option<u64>,
}

#[cfg(windows)]
impl PawnIoSensorReader {
    pub fn new() -> Option<Self> {
        // Attempt to load PawnIOLib.dll from standard installation path or system search path
        let lib = unsafe {
            libloading::Library::new(r"C:\Program Files\PawnIO\PawnIOLib.dll")
                .or_else(|_| libloading::Library::new("PawnIOLib.dll"))
        }
        .ok()?;

        let (open_fn, load_fn, exec_fn, close_fn) = unsafe {
            let open: PawnioOpenFn = *lib.get(b"pawnio_open\0").ok()?;
            let load: PawnioLoadFn = *lib.get(b"pawnio_load\0").ok()?;
            let exec: PawnioExecuteFn = *lib.get(b"pawnio_execute\0").ok()?;
            let close: PawnioCloseFn = *lib.get(b"pawnio_close\0").ok()?;
            (open, load, exec, close)
        };

        // Initialize MSR module
        let mut msr_handle = None;
        let mut tj_max = 100.0f32;
        let mut raw_msr_h: isize = 0;
        if unsafe { open_fn(&mut raw_msr_h) } == 0 {
            if unsafe { load_fn(raw_msr_h, INTEL_MSR_BIN.as_ptr(), INTEL_MSR_BIN.len()) } == 0 {
                msr_handle = Some(raw_msr_h);

                // Read IA32_TEMPERATURE_TARGET (0x1A2) for TjMax
                let in_buf = [0x1A2u64];
                let mut out_buf = [0u64; 1];
                let mut ret_size = 0usize;
                let res = unsafe {
                    exec_fn(
                        raw_msr_h,
                        c"ioctl_read_msr".as_ptr(),
                        in_buf.as_ptr(),
                        1,
                        out_buf.as_mut_ptr(),
                        1,
                        &mut ret_size,
                    )
                };
                if res == 0 {
                    let tj = ((out_buf[0] >> 16) & 0xFF) as f32;
                    if (50.0..=120.0).contains(&tj) {
                        tj_max = tj;
                    }
                }
            } else {
                unsafe { close_fn(raw_msr_h) };
            }
        }

        // Initialize Super I/O module
        let mut lpc_handle = None;
        let mut hwm_base = None;
        let mut raw_lpc_h: isize = 0;
        if unsafe { open_fn(&mut raw_lpc_h) } == 0 {
            if unsafe { load_fn(raw_lpc_h, LPC_IO_BIN.as_ptr(), LPC_IO_BIN.len()) } == 0 {
                // Initialize Super I/O under ISA bus mutex
                let _mutex_guard = acquire_isa_bus_mutex();
                let mut out_buf = [0u64; 1];
                let mut ret_size = 0usize;

                // 1. Select slot 0 (0x2E/0x2F)
                let in_slot = [0u64];
                let slot_res = unsafe {
                    exec_fn(
                        raw_lpc_h,
                        c"ioctl_select_slot".as_ptr(),
                        in_slot.as_ptr(),
                        1,
                        out_buf.as_mut_ptr(),
                        1,
                        &mut ret_size,
                    )
                };

                // 2. Enter ITE config mode (write 0x87, 0x01, 0x55, 0x55 to 0x2E)
                let enter_seq = [0x87u64, 0x01, 0x55, 0x55];
                let mut enter_ok = slot_res == 0;
                for &b in &enter_seq {
                    let in_outb = [0x2Eu64, b];
                    let r = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_pio_outb".as_ptr(),
                            in_outb.as_ptr(),
                            2,
                            out_buf.as_mut_ptr(),
                            0,
                            &mut ret_size,
                        )
                    };
                    if r != 0 {
                        enter_ok = false;
                        break;
                    }
                }

                // 3. Verify Chip ID (reg 0x20 and 0x21 == 0x8613)
                let mut chip_valid = false;
                if enter_ok {
                    let in_r20 = [0x20u64];
                    let r1 = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_superio_inb".as_ptr(),
                            in_r20.as_ptr(),
                            1,
                            out_buf.as_mut_ptr(),
                            1,
                            &mut ret_size,
                        )
                    };
                    let chip_msb = out_buf[0] & 0xFF;

                    let in_r21 = [0x21u64];
                    let r2 = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_superio_inb".as_ptr(),
                            in_r21.as_ptr(),
                            1,
                            out_buf.as_mut_ptr(),
                            1,
                            &mut ret_size,
                        )
                    };
                    let chip_lsb = out_buf[0] & 0xFF;

                    if r1 == 0 && r2 == 0 && chip_msb == 0x86 && chip_lsb == 0x13 {
                        chip_valid = true;
                    }
                }

                // 4. Select LDN 0x04 (Environment Controller) & read HWM base address
                let mut base_addr = None;
                if chip_valid {
                    let in_ldn = [0x07u64, 0x04u64];
                    let _ = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_superio_outb".as_ptr(),
                            in_ldn.as_ptr(),
                            2,
                            out_buf.as_mut_ptr(),
                            0,
                            &mut ret_size,
                        )
                    };

                    let in_r60 = [0x60u64];
                    let r_msb = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_superio_inb".as_ptr(),
                            in_r60.as_ptr(),
                            1,
                            out_buf.as_mut_ptr(),
                            1,
                            &mut ret_size,
                        )
                    };
                    let base_msb = out_buf[0] & 0xFF;

                    let in_r61 = [0x61u64];
                    let r_lsb = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_superio_inb".as_ptr(),
                            in_r61.as_ptr(),
                            1,
                            out_buf.as_mut_ptr(),
                            1,
                            &mut ret_size,
                        )
                    };
                    let base_lsb = out_buf[0] & 0xFF;

                    if r_msb == 0 && r_lsb == 0 {
                        let parsed_base = (base_msb << 8) | base_lsb;
                        if parsed_base != 0 && parsed_base != 0xFFFF {
                            base_addr = Some(parsed_base);
                        }
                    }

                    // 5. Call ioctl_find_bars to whitelist the HWM BAR
                    let dummy_in: [u64; 0] = [];
                    let _ = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_find_bars".as_ptr(),
                            dummy_in.as_ptr(),
                            0,
                            out_buf.as_mut_ptr(),
                            1,
                            &mut ret_size,
                        )
                    };

                    // 6. Exit ITE config mode (write 0x02 to reg 0x02)
                    let in_exit = [0x02u64, 0x02u64];
                    let _ = unsafe {
                        exec_fn(
                            raw_lpc_h,
                            c"ioctl_superio_outb".as_ptr(),
                            in_exit.as_ptr(),
                            2,
                            out_buf.as_mut_ptr(),
                            0,
                            &mut ret_size,
                        )
                    };
                }

                if let Some(base) = base_addr {
                    hwm_base = Some(base);
                    lpc_handle = Some(raw_lpc_h);
                } else {
                    unsafe { close_fn(raw_lpc_h) };
                }
            } else {
                unsafe { close_fn(raw_lpc_h) };
            }
        }

        if msr_handle.is_none() && lpc_handle.is_none() {
            return None;
        }

        Some(Self {
            _lib: lib,
            pawnio_execute: exec_fn,
            pawnio_close: close_fn,
            msr_handle,
            lpc_handle,
            tj_max,
            hwm_base,
        })
    }

    /// Reads live CPU package temperature via MSR 0x1B1 (`IA32_PACKAGE_THERM_STATUS`),
    /// falling back to MSR 0x19C (`IA32_THERM_STATUS`).
    pub fn read_cpu_temp(&self) -> Option<f32> {
        let msr_h = self.msr_handle?;
        let mut out_buf = [0u64; 1];
        let mut ret_size = 0usize;

        // Try IA32_PACKAGE_THERM_STATUS (0x1B1)
        let in_buf = [0x1B1u64];
        let res = unsafe {
            (self.pawnio_execute)(
                msr_h,
                c"ioctl_read_msr".as_ptr(),
                in_buf.as_ptr(),
                1,
                out_buf.as_mut_ptr(),
                1,
                &mut ret_size,
            )
        };
        if res == 0 {
            let val = out_buf[0];
            // Bit 31: Reading Valid
            if (val & 0x8000_0000) != 0 {
                let delta = ((val >> 16) & 0x7F) as f32;
                let temp = self.tj_max - delta;
                if (10.0..=120.0).contains(&temp) {
                    return Some(temp);
                }
            }
        }

        // Fallback to IA32_THERM_STATUS (0x19C)
        let in_buf_fallback = [0x19Cu64];
        let res_fb = unsafe {
            (self.pawnio_execute)(
                msr_h,
                c"ioctl_read_msr".as_ptr(),
                in_buf_fallback.as_ptr(),
                1,
                out_buf.as_mut_ptr(),
                1,
                &mut ret_size,
            )
        };
        if res_fb == 0 {
            let val = out_buf[0];
            if (val & 0x8000_0000) != 0 {
                let delta = ((val >> 16) & 0x7F) as f32;
                let temp = self.tj_max - delta;
                if (10.0..=120.0).contains(&temp) {
                    return Some(temp);
                }
            }
        }

        None
    }

    /// Reads live fan RPM from the ITE IT8613E Super I/O hardware monitor.
    ///
    /// Checks `FAN2` (`CPU_FAN` header on LattePanda Mu Ultra), falling back
    /// to `FAN1` or `FAN3` if `FAN2` reads `0` or `0xFFFF`.
    pub fn read_fan_rpm(&self) -> Option<u32> {
        let lpc_h = self.lpc_handle?;
        let base = self.hwm_base?;

        let _guard = acquire_isa_bus_mutex()?;

        let mut out_buf = [0u64; 1];
        let mut ret_size = 0usize;

        // Fan register mappings for IT8613E:
        // FAN2: LSB 0x0E, MSB 0x19 (CPU_FAN header on Mu Ultra)
        // FAN1: LSB 0x0D, MSB 0x18
        // FAN3: LSB 0x0F, MSB 0x1A
        let fan_regs: &[(u64, u64)] = &[(0x0E, 0x19), (0x0D, 0x18), (0x0F, 0x1A)];

        for &(reg_lsb, reg_msb) in fan_regs {
            // Write LSB register index to base + 5
            let in_idx_lsb = [base + 5, reg_lsb];
            let r1 = unsafe {
                (self.pawnio_execute)(
                    lpc_h,
                    c"ioctl_pio_outb".as_ptr(),
                    in_idx_lsb.as_ptr(),
                    2,
                    out_buf.as_mut_ptr(),
                    0,
                    &mut ret_size,
                )
            };
            if r1 != 0 {
                continue;
            }

            // Read LSB from base + 6
            let in_data_port = [base + 6];
            let r2 = unsafe {
                (self.pawnio_execute)(
                    lpc_h,
                    c"ioctl_pio_inb".as_ptr(),
                    in_data_port.as_ptr(),
                    1,
                    out_buf.as_mut_ptr(),
                    1,
                    &mut ret_size,
                )
            };
            if r2 != 0 {
                continue;
            }
            let lsb = out_buf[0] & 0xFF;

            // Write MSB register index to base + 5
            let in_idx_msb = [base + 5, reg_msb];
            let r3 = unsafe {
                (self.pawnio_execute)(
                    lpc_h,
                    c"ioctl_pio_outb".as_ptr(),
                    in_idx_msb.as_ptr(),
                    2,
                    out_buf.as_mut_ptr(),
                    0,
                    &mut ret_size,
                )
            };
            if r3 != 0 {
                continue;
            }

            // Read MSB from base + 6
            let r4 = unsafe {
                (self.pawnio_execute)(
                    lpc_h,
                    c"ioctl_pio_inb".as_ptr(),
                    in_data_port.as_ptr(),
                    1,
                    out_buf.as_mut_ptr(),
                    1,
                    &mut ret_size,
                )
            };
            if r4 != 0 {
                continue;
            }
            let msb = out_buf[0] & 0xFF;

            let raw = (lsb as u16) | ((msb as u16) << 8);
            if raw > 0x3F && raw < 0xFFFF {
                let rpm = (1_350_000.0 / (raw as f64 * 2.0)).round() as u32;
                if (100..=15_000).contains(&rpm) {
                    return Some(rpm);
                }
            }
        }

        None
    }
}

#[cfg(windows)]
impl Drop for PawnIoSensorReader {
    fn drop(&mut self) {
        if let Some(h) = self.msr_handle.take() {
            unsafe { (self.pawnio_close)(h) };
        }
        if let Some(h) = self.lpc_handle.take() {
            unsafe { (self.pawnio_close)(h) };
        }
    }
}

#[cfg(not(windows))]
pub struct PawnIoSensorReader;

#[cfg(not(windows))]
impl PawnIoSensorReader {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn read_cpu_temp(&self) -> Option<f32> {
        None
    }

    pub fn read_fan_rpm(&self) -> Option<u32> {
        None
    }
}
