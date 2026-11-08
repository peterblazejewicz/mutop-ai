# Adversarial Primary-Source Verification: `mutop-ai` Temperature, Power & Throttle Readings on LattePanda Mu Ultra (`mu`)

**Date:** 2026-11-08  
**Target Hardware:** LattePanda Mu Ultra (`DFR1295` Compute Module + `DFR1293` Mini Carrier Board, Host `mu`)  
**Firmware & OS:** BIOS `SBCLNLCXR120-A` (`ALASKA - 1072009`), Windows 11 Pro 26H2 (`10.0.26300.9711`)  
**Drivers:** PawnIO `v2.2.0`, Intel Arc 140V `32.0.101.9033`, Intel AI Boost NPU 4 `32.0.100.5540`  
**Scope:** Primary-source verification of claims regarding `mutop-ai` readings in `m6b9-e2b` (2026-11-08), KB claims (`wiki/tools/mutop-ai.md`, `wiki/devices/lattepanda-mu-ultra.md`), and adversarial review of our own fact-checks and telemetry implementation.

---

## 1. Executive Summary Matrix (Claim-by-Claim Verdict)

| # | Claim Under Review | Verdict | Primary-Source Finding & Adversarial Notes |
|---|---|---|---|
| **1** | **Other Agent:** *"What `mutop-ai` recorded in our runs (first sample of `m6b9-e2b` on 2026-11-08): `power_thermals: package_w 3.00, core_pp0_w 0.55, gpu_pp1_w 0.0, dram_w 0.010, power_above_idle_w 0.0, idle_baseline_w 5.49, ac_online true, battery_present false, effective_power_mode "Balanced"`, `gpu.throttling: ... false`, `ai_diagnostics: ... false`. It has power figures and throttle flags, but no temperature field."* | **VERIFIED (with critical adversarial nuance on `gpu.throttling`)** | Exact match to line 1 of `C:\develop\device-commander\m6b\runs\m6b9-e2b\mutop.ndjson` (`timestamp_unix: 1794113307`). `PowerThermalSnapshot` (`src/snapshot/types.rs:166-196`) has 7 optional temperature fields (`acpi_thermal_zone_c`, `gpu_c`, `nvme_composite_c`, `nvme_asic_c`, `nvme_nand_c`, `cpu_package_dts_c`, `cpu_core_dts_c`), all marked `#[serde(skip_serializing_if = "Option::is_none")]`. When all 7 evaluate to `None`, `serde_json` omits their keys.<br>**Adversarial Catch:** While `power_thermals` RAPL watts (`package_w`, `core_pp0_w`, `gpu_pp1_w`, `dram_w`) and `cpu.is_throttled` (`CallNtPowerInformation`) are live unprivileged hardware readings, `gpu.throttling` (`all false`) and `ai_diagnostics.nvme_thermal_throttling` (`false`) in `m6b9-e2b` are **fallback defaults (`false`)**, not active sensor confirmations (see §4). |
| **2** | **Other Agent:** *"My guess at why the temperature is missing: our runs start `mutop-ai` with limited rights, and reading the CPU temperature registers needs administrator rights. I haven't tested this."* | **VERIFIED (Tested Live on `mu`)** | `m6b9-e2b` ran via `run-interactive.ps1` with `-RunLevel Limited` (`host.privilege_tier: "Tier1Unprivileged"`). Live testing on `mu` under a `Limited` token (`TokenIsElevated = 0`) vs. `Elevated` token (`TokenIsElevated = 1`) proves:<br>1. **CPU Package DTS (`cpu_package_dts_c`)** & **Fan (`cpu_fan_rpm`)** use `PawnIOLib.dll` (`MSR 0x1B1` / `IT8613E`). Unprivileged `pawnio_open()` fails with `HRESULT 0x80070005` (`E_ACCESSDENIED`). Elevated returns `51.0 °C` and `2220 RPM`.<br>2. **NVMe SMART Temps (`nvme_composite_c`, `nvme_asic_c`, `nvme_nand_c`)** open `\\.\PhysicalDrive0` with `GENERIC_READ` (`src/hardware/storage.rs:195-204`). Unprivileged `CreateFileW` fails with `ERROR_ACCESS_DENIED (5)`. Elevated returns `32.0 °C` / `38.0 °C` / `32.0 °C`.<br>3. **ACPI ThermalZone (`acpi_thermal_zone_c`)** is readable unprivileged via PDH, but returns a static firmware dummy constant (`3010` dK = `27.85 °C`) that `parse_acpi_thermal_reading()` (`src/hardware/power.rs:15-25`) intentionally discards to `None`.<br>4. **Arc 140V GPU Temp (`gpu_c`)** is unpopulated on Lunar Lake (`bSupported = 0` in IGCL, plus an FFI struct size mismatch in `src/hardware/gpu.rs:229-254`). |
| **3** | **Other Agent:** *"`wiki/tools/mutop-ai.md` line 47 (Gotchas): 'ACPI ThermalZone exposes a constant dummy sensor reading `0x0BB8` (28.0 °C) on LattePanda Mu; telemetry parsers must explicitly discard static sensors.' The same claim is in `wiki/devices/lattepanda-mu-ultra.md` line 34. No finding in `findings/` backs it, so it is unverified. The schema's rule that numbers live only in findings is broken here."* | **VERIFIED (Plus Adversarial Math & AML Correction)** | 1. **KB Schema Rule Confirmed:** Neither `wiki/tools/mutop-ai.md:47` nor `wiki/devices/lattepanda-mu-ultra.md:34` links a `findings/` file for the ACPI dummy sensor numbers, violating `SCHEMA.md` Rule 4.<br>2. **Adversarial Math & Firmware AML Discovery:** The KB notes conflate **two different ACPI constants** that both exist in the `mu` BIOS (`SBCLNLCXR120-A`) AML tables:<br>• `0x0BB8` hex = **`3000` tenths of Kelvin** (`300.0 K` = **`26.85 °C`**, *not* `28.0 °C`), located in SSDT `Ther_Rvp` (`\_TZ.TZ00.PTMP` and `_TMP` when `ETMD == 0`).<br>• `0x0BC2` hex = **`3010` tenths of Kelvin** (`301.0 K` = **`27.85 °C`**, integer `301 K`, which rounds to **`28 °C`** in `{:.0}°C` formatting), located in `DSDT` (`\_SB.PC00.LPCB.MXTP` fallback when `H_EC.DUMY` is absent).<br>On live `mu` hardware (`ETMD == 1`, no `H_EC`), `\_TZ.TZ00._TMP` calls `MXTP()` and returns **`0x0BC2` (`3010` dK = `27.85 °C`)**, which `mutop-ai` filters out via `(2999.5..=3010.5).contains(&raw_k_tenths)`. |
| **4** | **Other Agent:** *"The only measured temperature is the 2026-11-05 stress test: a median of 57 °C over 60 s. That run was probably elevated."* | **PARTIALLY TRUE IN KB, FALSE IN REPO** | 1. **Was the 2026-11-05 run elevated?** **Yes** (`host.privilege_tier = "Tier2ElevatedAdmin"` with `PawnIO` active, reading `MSR 0x1B1` relative to `MSR 0x1A2`).<br>2. **Is 57 °C (and 81 °C peak / 100 °C TjMax) the only temperature promoted to `D:\develop\kb\findings\`?** **Yes** — inside `$KB_ROOT/findings/`, only `2026-11-05-mu-tri-accelerator-concurrent-workflows-and-thermals.md` (`57.0 °C` median, `81.0 °C` peak) and `2026-11-05-mu-disputed-claims-reverification-msr-aperture-ecore-gpu.md` (`MSR 0x1A2 TjMax = 100.0 °C`) record temperature numbers.<br>3. **Is it the only measured temperature in `mutop-ai`?** **No** — `D:\develop\mutop-ai\docs\benchmark-results\` contains **27+ distinct elevated benchmark runs** across Waves 1–4 with full `cpu_temp_median_c` (`43.0–57.0 °C`), `cpu_temp_max_c` (`54.0–95.0 °C`), and `fan_median_rpm` (`2,766–4,167 RPM`), plus the 2026-11-04 pre-filter ACPI reading (`27.85 °C`) in `docs/mu-verification-results.md:35`. |

---

## 2. Verification of the `m6b9-e2b` Snapshot & Privilege Tier Behavior

### 2.1 Primary Source: `m6b9-e2b` First Sample vs. Elevated Snapshot on `mu`
Inspecting line 1 of `C:\develop\device-commander\m6b\runs\m6b9-e2b\mutop.ndjson` on `mu` confirms the other agent's quote verbatim:

```json
{
  "timestamp_unix": 1794113307,
  "timestamp_unix_ms": 1794113307717,
  "interval_ms": 26.437700000000003,
  "marker": "idle",
  "host": {
    "board": "LattePanda Mu Ultra (DFR1295 + DFR1293)",
    "soc": "Intel Core Ultra 7 256V (Lunar Lake)",
    "hostname": "MU",
    "os_build": "Windows 11 Build 10.0.26300",
    "architecture": "x86_64",
    "privilege_tier": "Tier1Unprivileged",
    "total_ai_tops": 115,
    "npu_tops": 47,
    "gpu_tops": 64,
    "cpu_tops": 5
  },
  "power_thermals": {
    "package_w": 3.0003056558363417,
    "core_pp0_w": 0.5534544438714114,
    "gpu_pp1_w": 0.0,
    "dram_w": 0.010195203713254254,
    "power_above_idle_w": 0.0,
    "idle_baseline_w": 5.49,
    "ac_online": true,
    "battery_present": false,
    "effective_power_mode": "Balanced"
  }
}
```

As documented in `D:\develop\device-commander\docs\experiments\2026-11-08-m6b9-trials.md:11`, every program in `m6b9` ran in Session 1 through a one-shot interactive scheduled task with `-RunLevel Limited` (`run-interactive.ps1`). Consequently, `detect_windows_privilege()` (`src/hardware/facts.rs:96-122`) observed `TokenIsElevated == 0` and set `host.privilege_tier = "Tier1Unprivileged"`.

### 2.2 Controlled Live Experiment on `mu`: `Limited` (`Tier1Unprivileged`) vs. `Elevated` (`Tier2ElevatedAdmin`)
We executed a controlled side-by-side test on `mu` (`2026-11-08`) comparing a Session 1 scheduled task with `-RunLevel Limited` against an elevated Administrator session:

| Telemetry / API Probe | `-RunLevel Limited` (`TokenIsElevated = 0`) | Elevated Admin (`TokenIsElevated = 1`) | Root Cause & Source Reference |
|---|---|---|---|
| `host.privilege_tier` | `"Tier1Unprivileged"` | `"Tier2ElevatedAdmin"` | `GetTokenInformation(..., TokenElevation)` (`src/hardware/facts.rs:96-122`) |
| `PawnIOLib.dll` `pawnio_open(&mut h)` | `0x80070005` (`E_ACCESSDENIED`), `h = 0` | `0x00000000` (`S_OK`), `h = 388` | `\\.\PawnIO` device object DACL requires Administrator (`src/hardware/pawnio.rs:99-121`) |
| `power_thermals.cpu_package_dts_c` | *(omitted / `None`)* | `51.0` (°C) | Read from `MSR 0x1B1` (`IA32_PACKAGE_THERM_STATUS`) relative to `MSR 0x1A2` (`TjMax = 100 °C`) via PawnIO (`src/hardware/pawnio.rs:123-143, 340-395`) |
| `power_thermals.cpu_fan_rpm` | *(omitted / `None`)* | `2220` (RPM) | Read from ITE `IT8613E` Super I/O (`LDN 0x04`, HWM base `0x0A40`, `FAN2` regs `0x0E`/`0x19`) via PawnIO (`src/hardware/pawnio.rs:397-497`) |
| `CreateFileW("\\\\.\\PhysicalDrive0", GENERIC_READ)` | `INVALID_HANDLE_VALUE (-1)`, `GetLastError() = 5` (`ERROR_ACCESS_DENIED`) | Valid handle, `GetLastError() = 0` | Raw disk handle with `GENERIC_READ` requires Admin (`src/hardware/storage.rs:193-208`) |
| `power_thermals.nvme_composite_c` / `nvme_asic_c` / `nvme_nand_c` | *(omitted / `None`)* | `32.0` / `38.0` / `32.0` (°C) | Read via `IOCTL_STORAGE_QUERY_PROPERTY` (`PropertyId = 50` NVMe Log `0x02` & `PropertyId = 52` `StorageDeviceTemperatureProperty`, `src/hardware/storage.rs:222-350`) |
| `power_thermals.acpi_thermal_zone_c` | *(omitted / `None`)* | *(omitted / `None`)* | PDH `\Thermal Zone Information(*)\High Precision Temperature` succeeds in both tiers and returns `3010.0` dK (`27.85 °C`), which `parse_acpi_thermal_reading()` filters out (`src/hardware/power.rs:15-25`) |
| `power_thermals.package_w` / `core_pp0_w` / `gpu_pp1_w` / `dram_w` | `7.92 W` / `4.38 W` / `0.34 W` / `0.065 W` | `4.43 W` / `1.80 W` / `0.0 W` / `0.022 W` | Works in **both** tiers because Windows Performance Counters read `emi.sys` (`\Energy Meter(RAPL_Package0_*)\Power`, `src/hardware/pdh.rs:84-87`) on behalf of unprivileged users |

---

## 3. Primary-Source Disassembly of the ACPI ThermalZone Dummy Readings (`0x0BB8` vs. `0x0BC2`)

### 3.1 Live WMI and Performance Counter Values on `mu`
Querying all three Windows ACPI thermal zone surfaces live on `mu` (BIOS `SBCLNLCXR120-A`) returns:
1. **`root/wmi:MSAcpi_ThermalZoneTemperature`**:
   - `InstanceName`: `"ACPI\\ThermalZone\\TZ00_0"`
   - `CurrentTemperature`: **`3010`** (tenths of Kelvin = **`0x0BC2`**)
2. **`Win32_PerfFormattedData_Counters_ThermalZoneInformation`** & **`Win32_PerfRawData_Counters_ThermalZoneInformation`** (the backing source for PDH `\Thermal Zone Information(\_TZ.TZ00)\High Precision Temperature`):
   - `Name`: `"\\_TZ.TZ00"`
   - `HighPrecisionTemperature`: **`3010`** (tenths of Kelvin = **`0x0BC2`**)
   - `Temperature`: **`301`** (integer Kelvin)

### 3.2 Exact ACPI AML Bytecode Disassembly from `mu` Firmware Tables
By reading the live firmware tables on `mu` via `kernel32!GetSystemFirmwareTable` and `HKLM\HARDWARE\ACPI`, we located the exact AML definitions for `\_TZ.TZ00._TMP`:

1. **SSDT `Ther_Rvp` (`HKLM\HARDWARE\ACPI\SSDH\ALASKA\Ther_Rvp\00001000\00000000`, length `14,678` bytes)**:
   - **Offset `995` (`0x03E3`)**: `08 45 54 4D 44 01`
     ```asl
     Scope (\_TZ) {
         Name (ETMD, 0x01)
     }
     ```
   - **Offset `2132` (`0x0854`)**: `5B 85 89 0A 03 54 5A 30 30 08 50 54 4D 50 0B B8 0B`
     ```asl
     ThermalZone (TZ00) {
         Name (PTMP, 0x0BB8) // 3000 tenths of Kelvin = 300.0 K = 26.85 °C
     ```
   - **Offset `2466` (`0x09A2`)**: `14 25 5F 54 4D 50 08 A0 0A 92 45 54 4D 44 A4 0B B8 0B A4 5C 2F 04 5F 53 42 5F 50 43 30 30 4C 50 43 42 4D 58 54 50`
     ```asl
     Method (_TMP, 0, Serialized) {
         If (LNot (ETMD)) {
             Return (0x0BB8) // 3000 tenths of Kelvin = 300.0 K = 26.85 °C
         }
         Return (\_SB.PC00.LPCB.MXTP ())
     }
     ```

2. **DSDT (`HKLM\HARDWARE\ACPI\DSDT\ALASKA\A_M_I_\01072009\00000000`, length `144,863` bytes)**:
   - **Offset `140,841` (`0x22629`)**: `14 44 04 4D 58 54 50 08 A0 38 5B 12 5C 2F 05 5F 53 42 5F 50 43 30 30 4C 50 43 42 48 5F 45 43 44 55 4D 59 00 70 5E 2F 03 48 5F 45 43 44 55 4D 59 45 52 4D 54 60 72 0B AC 0A 77 60 0A 0A 00 60 A4 60 A4 0B C2 0B`
     ```asl
     Method (MXTP, 0, Serialized) {
         If (CondRefOf (\_SB.PC00.LPCB.H_EC.DUMY)) {
             Store (\_SB.PC00.LPCB.H_EC.DUMY.ERMT, Local0)
             Add (0x0AAC, Multiply (Local0, 0x0A), Local0)
             Return (Local0)
         }
         Return (0x0BC2) // 3010 tenths of Kelvin = 301.0 K = 27.85 °C
     }
     ```

### 3.3 Why the KB Text Had `"0x0BB8 (28.0 °C)"` (Adversarial Fact-Check of Our Own Docs)
- In the `mu` BIOS, **both** dummy constants exist in the thermal zone path:
  - `0x0BB8` = `3000` tenths of Kelvin = $300.0\text{ K} - 273.15 = \mathbf{26.85\,^\circ\text{C}}$ (used when `ETMD == 0`).
  - `0x0BC2` = `3010` tenths of Kelvin = $301.0\text{ K} - 273.15 = \mathbf{27.85\,^\circ\text{C}}$ (used when `ETMD == 1` and `\_SB.PC00.LPCB.H_EC.DUMY` is absent, which is the actual runtime path on `mu`!).
- Note also how ACPI integer arithmetic in `MXTP` defines $0\,^\circ\text{C}$ as `0x0AAC = 2732` tenths of Kelvin ($273.2\text{ K}$); relative to `2732`, `3000` is $26.8\,^\circ\text{C}$ and `3010` is $27.8\,^\circ\text{C}$, while in `mutop-ai`'s TUI (`src/ui.rs:778`, `format!("{:.0}°C", t)`), `27.85 °C` rounds to **`28°C`**.
- `src/hardware/power.rs:12-18` accurately comments and filters **both** (`3000` and `3010` tenths of Kelvin, `26.85°C / 27.85°C`, range `2999.5..=3010.5`), and `docs/mu-verification-results.md:35` (2026-11-04, before the filter was added) accurately recorded the live reading as **`27.85 °C`**.
- However, `README.md:26` and the commit message of `5539c96` compressed this into `0x0BB8 / 3000-3010 tenths of Kelvin = 28°C`, and the KB notes (`wiki/tools/mutop-ai.md:47` and `wiki/devices/lattepanda-mu-ultra.md:34`) further conflated them into `0x0BB8 (28.0 °C)` without a backing finding in `findings/`.

---

## 4. Adversarial Review of `gpu.throttling` and `ai_diagnostics` in `mutop-ai`

The other agent noted:
> *"gpu.throttling: power_limited false, temp_limited false, current_limited false, voltage_limited false*  
> *ai_diagnostics: nvme_thermal_throttling false, gpu_or_cpu_power_throttling false*  
> *It has power figures and throttle flags, but no temperature field."*

An adversarial audit of `src/hardware/gpu.rs`, `src/hardware/storage.rs`, and `src/hardware/diagnostics.rs` reveals **three important caveats** about those boolean flags that any consumer of `mutop-ai` data must know:

1. **`gpu.throttling` (`power_limited`, `temp_limited`, `current_limited`, `voltage_limited`) is currently always `false` due to an IGCL FFI struct layout mismatch in `src/hardware/gpu.rs:229-254`**:
   - In `src/hardware/gpu.rs:229-236`, `CtlInitArgs` is 24 bytes and passes `supported_version = 1`.
   - In Intel's `ControlLib.dll` (`igcl_api.h` on driver `32.0.101.9033`), `ctl_init_args_t` is **36 bytes** (includes `ctl_application_id_t ApplicationUID` of 16 bytes) and expects `AppVersion = CTL_MAKE_VERSION(1, 0) = 0x00010000`.
   - Because `size = 24` is passed, `ctlInit` returns `0x40000009` (`CTL_RESULT_ERROR_UNSUPPORTED_VERSION`), `try_read_igcl()` returns `None`, and `collect_windows_gpu()` (`src/hardware/gpu.rs:183-186`) falls back to `(0, None, None, GpuThrottlingFlags::default())` where all four booleans default to `false` and `clock_mhz` defaults to `0`.
   - Furthermore, inside `ctl_power_telemetry_t`, each metric is a 24-byte `ctl_oc_telemetry_item_t` (`bSupported`, `units`, `type`, `value`). Probing `ctlPowerTelemetryGet` live on `mu` with the corrected 36-byte `ctl_init_args_t` shows:
     - `timeStamp` (item 0): `bSupported = 1`, live uptime seconds.
     - `gpuEnergyCounter` (item 1): `bSupported = 1`, cumulative Joules (`85.67 J`).
     - `gpuCurrentClockFrequency` (item 3): `bSupported = 1`, live GPU clock (`1550.0 MHz`).
     - **`gpuCurrentTemperature` (item 4): `bSupported = 0`, `value = 0.0`** — Intel's Arc 140V driver on Lunar Lake does **not** expose a separate GPU temperature via IGCL `ctlPowerTelemetryGet` (even when elevated), because the Xe2 GPU shares the main SoC compute die (`MSR 0x1B1` package DTS).
2. **`ai_diagnostics.nvme_thermal_throttling: false` in unprivileged runs is a vacuously false default**:
   - In `src/hardware/diagnostics.rs:73-77`, `nvme_thermal_throttling` is computed from `storage.smart_health` (`warning_throttle_minutes > 0 || critical_throttle_minutes > 0 || composite_temp_c >= 70.0`).
   - When running unprivileged (`Tier1Unprivileged`), `storage.smart_health` is `None` (`PhysicalDrive0` returns `ERROR_ACCESS_DENIED`), so `nvme_thermal_throttling` evaluates to `false` by default rather than `null`/unknown.
3. **`cpu.is_throttled` and `ai_diagnostics.gpu_or_cpu_power_throttling` (CPU side) ARE live in unprivileged runs**:
   - `CallNtPowerInformation(ProcessorInformation /* 11 */)` (`src/hardware/cpu.rs:202-239`) works unprivileged and checks `per_core_limit_mhz[i] < per_core_max_mhz[i]` across all 8 cores.

---

## 5. Complete Inventory of Measured Temperatures in `D:\develop\mutop-ai`

Contrary to the assumption that the 2026-11-05 60-second stress test (`57.0 °C` median) is the only measured temperature, `D:\develop\mutop-ai` contains **28 live hardware thermal measurements** on `mu`:

| Date | Dataset / Source File | Scenario / Workload | Privilege / Sensor Source | Measured Temperature(s) | Measured Fan Speed |
|---|---|---|---|---|---|
| 2026-11-04 | `docs/mu-verification-results.md:35` | Idle / nominal board verification (pre-filter) | PDH `\Thermal Zone Information` (`\_TZ.TZ00`) | **27.85 °C** (`3010` dK static dummy) | N/A |
| 2026-11-04 | `docs/mu-multi-engine-benchmark-report.md:120` | Wave 1 Scenario 1A (`FunctionGemma-270M-INT8` NPU, 50 turns) | Elevated (`PawnIO` `MSR 0x1B1`) | **45.0 °C** median | **2,860 RPM** median |
| 2026-11-04 | `docs/mu-multi-engine-benchmark-report.md:136` | Wave 1 Scenario 1B (`EmbeddingGemma-300M` NPU) | Elevated (`PawnIO` `MSR 0x1B1`) | **46.0 °C** median | N/A |
| 2026-11-04 | `docs/benchmark-results/wave2/scenario_1d_matrix.json` | Wave 2 Scenario 1D (Whisper 4-Tier × 4 Engines = **16 runs**) | Elevated (`PawnIO` `MSR 0x1B1` + `IT8613E`) | Medians: **43.0 – 54.0 °C**<br>Peaks: **54.0 – 95.0 °C** | Medians: **2,766 – 4,018 RPM**<br>Peaks: **3,140 – 5,315 RPM** |
| 2026-11-04 | `docs/benchmark-results/wave2/verify_ecore_whisper.json` | Wave 2 OpenVINO `PCORE_ONLY` vs `ECORE_ONLY` (**4 runs**) | Elevated (`PawnIO` `MSR 0x1B1` + `IT8613E`) | Medians: **47.0 – 50.0 °C**<br>Peaks: **57.0 – 81.0 °C** | Medians: **2,987 – 3,125 RPM** |
| 2026-11-04 | `docs/benchmark-results/wave2/scenario_1f_summary.json` | Wave 2 Scenario 1F (Piper Neural TTS on P-cores & LP-E cores, **2 runs**) | Elevated (`PawnIO` `MSR 0x1B1` + `IT8613E`) | **46.0 °C** median (`55.0 °C` / `54.0 °C` peak) | **3,125 / 3,140 RPM** median |
| 2026-11-04 | `docs/benchmark-results/wave3/scenario_2a_summary.json` | Wave 3 Scenario 2A (10-turn interactive voice turn concurrency) | Elevated (`PawnIO` `MSR 0x1B1` + `IT8613E`) | **44.0 °C** median / **78.0 °C** peak | **2,778 RPM** median / **5,273 RPM** peak |
| 2026-11-04 | `docs/benchmark-results/wave3/scenario_2b_summary.json` | Wave 3 Scenario 2B (60s sustained tri-accelerator saturation) | Elevated (`PawnIO` `MSR 0x1B1` + `IT8613E`) | **57.0 °C** median / **81.0 °C** peak | **4,167 RPM** median / **5,720 RPM** peak |
| 2026-11-05 | `docs/benchmark-results/wave4/scenario_3a_3b_openvino.json` | Wave 4 Scenario 3A Part 4 (`NPU_TURBO` default / turbo / efficient, **3 runs**) | Elevated (`PawnIO` `MSR 0x1B1` + `IT8613E`) | Medians: **44.0, 44.0, 47.0 °C**<br>Peaks: **76.0, 74.0, 76.0 °C** | Medians: **2,935, 2,813, 2,974 RPM** |
| 2026-11-05 | `docs/benchmark-results/wave4/verify_kb_disputes.json:18` | Wave 4 Direct MSR `0x1A2` (`IA32_TEMPERATURE_TARGET`) verification | Elevated (`PawnIO` `MSR 0x1A2`) | **TjMax = 100.0 °C** (`msr_0x1a2_tjmax_c: 100`) | N/A |
| 2026-11-08 | Live verification on `mu` (§2.2 above) | Idle elevated snapshot (`mutop-ai.exe --json`) | Elevated (`PawnIO` + `PhysicalDrive0`) | CPU DTS: **51.0 °C**<br>NVMe Composite/ASIC/NAND: **32.0 / 38.0 / 32.0 °C** | **2,220 RPM** |

---

## 6. Concrete Recommendations for `mutop-ai` and the Knowledge Base (`$KB_ROOT`)

### 6.1 For `mutop-ai` (Tool / Codebase)
1. **Explicit Unprivileged Thermal Reason / Diagnostic or Null Serialization**:
   - Currently, `PowerThermalSnapshot` omits `None` fields (`#[serde(skip_serializing_if = "Option::is_none")]`), while `host.privilege_tier` is `"Tier1Unprivileged"`.
   - To prevent downstream benchmark scripts/agents from wondering why temperature fields are absent when `mutop-ai` is started with `-RunLevel Limited`, we can either:
     - Add an explicit `thermal_status: Option<String>` (or `unprivileged_sensors_unavailable: bool` / diagnostic note) in `PowerThermalSnapshot` when `privilege_tier == Tier1Unprivileged` and `cpu_package_dts_c.is_none()`, e.g., `"unavailable (Tier1Unprivileged: CPU DTS MSR 0x1B1 and NVMe SMART require Administrator elevation; ACPI TZ00 static dummy 3010 dK / 27.85 °C filtered)"`, **and/or**
     - Serialize `null` instead of dropping key names in JSON so the schema keys are always visible.
2. **Fix IGCL `CtlInitArgs` (`36` bytes, `AppVersion = 0x00010000`) and `CtlPowerTelemetry` (`24`-byte `ctl_oc_telemetry_item_t` items) in `src/hardware/gpu.rs`**:
   - Fixing the struct layout immediately restores live unprivileged Arc 140V `clock_mhz` (`1550 MHz`), `gpuEnergyCounter` (`Joules` $\to$ `power_w`), and real `gpu.throttling` limiter flags via `ControlLib.dll`.

### 6.2 For `$KB_ROOT` (`D:\develop\kb`)
1. **Promote a Dedicated Finding (`findings/2026-11-08-mu-acpi-dummy-thermal-zone-and-elevation-gating.md`)**:
   - Record the exact primary-source numbers from `mu` (BIOS `SBCLNLCXR120-A`):
     - `\_TZ.TZ00._TMP` live readout (`MSAcpi_ThermalZoneTemperature` & PDH `HighPrecisionTemperature`): **`3010` tenths of Kelvin (`0x0BC2` = `301.0 K` = `27.85 °C`)**.
     - SSDT `Ther_Rvp` (`PTMP` & `ETMD == 0` branch): **`3000` tenths of Kelvin (`0x0BB8` = `300.0 K` = `26.85 °C`)**.
     - Unprivileged (`Tier1Unprivileged`, `-RunLevel Limited`) vs. Elevated (`Tier2ElevatedAdmin`) sensor availability (`pawnio_open` `0x80070005`, `PhysicalDrive0` `ERROR_ACCESS_DENIED = 5`, elevated `MSR 0x1B1 = 51.0 °C`, `IT8613E FAN2 = 2220 RPM`, `NVMe SMART = 32.0 / 38.0 / 32.0 °C`).
2. **Update `wiki/tools/mutop-ai.md` (line 47) and `wiki/devices/lattepanda-mu-ultra.md` (line 34)**:
   - Link the new finding in `## Measured` and `## Gotchas` so `SCHEMA.md` Rule 4 ("Numbers live only in `findings/`") is strictly satisfied, and correct the conflated `"0x0BB8 (28.0 °C)"` text to accurately state **`0x0BC2` (`3010` dK = `27.85 °C` live in `MXTP`, rounding to `28 °C`) and `0x0BB8` (`3000` dK = `26.85 °C` in `Ther_Rvp` when `ETMD == 0`)**.
