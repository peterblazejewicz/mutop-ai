# mutop-ai

Hardware telemetry and AI monitor for the **LattePanda Mu Ultra** (DFR1295 compute module with Intel Core Ultra 7 256V Lunar Lake SoC + DFR1293 Mini Carrier Board).

## Architecture & Capabilities

- **115 AI TOPS Telemetry**: Intel AI Boost NPU 4 (47 TOPS MCDM), Intel Arc 140V GPU (64 TOPS Xe2-LPG, 8 cores), and Lunar Lake CPU (4 TOPS).
- **Core Microarchitecture Placement**: 4× Lion Cove P-cores (`0..3`) vs 4× Skymont LP-E cores (`4..7`), thread placement, EcoQoS, and frequency limiting/throttling.
- **Non-Double-Counting 16 GB MoP UMA Model**: Reconciles pure CPU working set, Arc GPU shared memory, and NPU shared memory against the 16384 MB physical memory pool and BIOS aperture.
- **RAPL Power Rails & Thermals**: Package (`PKG`), CPU (`PP0`), GPU (`PP1`), and DRAM power in Watts; ACPI, GPU, and NVMe temperatures.
- **Storage & Networking**: NVMe PCIe Gen4 x4 throughput, IOPS, latency, and SMART health; 2.5GbE, Intel BE201 Wi-Fi 7 (EHT/MLO), and local AI loopback sockets (`llama-server:8080`, `MCP:50051/8081`).

## Windows 11 Privilege Matrix

| Tier | Required Privileges | Accessible Telemetry Surfaces | Unavailable Behavior |
|:-----|:--------------------|:------------------------------|:---------------------|
| **Tier 1: Unprivileged** | Normal user process | `NtQuerySystemInformation`, DXCore / DXGI adapter enumeration, PDH engine & disk counters, `GetIfTable2`, `wlanapi.dll`, TCP extended tables, `NtQueryTimerResolution` | Baseline operation; higher-tier fields return `None` |
| **Tier 2: Elevated Admin** | Run as Administrator (`SeDebugPrivilege`) | Storage IOCTLs (`IOCTL_STORAGE_QUERY_PROPERTY` NVMe SMART Health Log `0x02`, PCIe link negotiation), elevated thread query | Link degradation flags and SMART temps omitted if unprivileged |
| **Tier 3: Signed Driver (Optional)** | Microsoft WHQL-Signed PawnIO Driver (`winget install namazso.PawnIO`) | Direct CPU Package/Core DTS temperatures via MSR `0x1B1`, Carrier Fan RPM (`CPU_FAN` header on ITE IT8613E Super I/O) | Unprivileged user-mode fallback: suppresses dummy ACPI constant (`28°C`), reports NVMe SMART temp, and displays `Fan: Auto` (autonomous hardware PWM) |

## Optional Hardware Driver: Fan Speed & CPU DTS Thermals (PawnIO)

`mutop-ai` operates in unprivileged user mode without kernel drivers by default. Standard execution provides RAPL power metering, UMA memory breakdown, thread-to-core placement, NPU/GPU duty cycles, and NVMe SMART thermals.

### Carrier Fan RPM & ACPI Dummy Sensor Suppression
On the LattePanda Mu Ultra (`DFR1295` compute module with Intel Core Ultra 7 256V + `DFR1293` mini carrier board), carrier fan tachometer feedback is managed by an onboard **ITE IT8613E Super I/O** controller connected via Low Pin Count (LPC) I/O ports. Furthermore, the carrier ACPI DSDT/SSDT table lacks an Embedded Controller (`ETMD == 0`), causing the firmware ACPI thermal zone to return a hardcoded static dummy constant `0x0BB8` (`3000`–`3010` tenths of Kelvin = `26.85°C`–`27.85°C`, rendered as `28°C`). `mutop-ai` suppresses this fake ACPI reading automatically.

### Live Telemetry with Signed PawnIO Driver
To read live **CPU Package Digital Thermal Sensor (DTS)** and **Carrier Fan RPM**, install the optional Microsoft WHQL-signed **PawnIO** driver:

```powershell
winget install namazso.PawnIO
```

- **HVCI, VBS & Secure Boot Compatible**: PawnIO is a modern, Microsoft-signed scriptable driver that runs sandboxed Pawn bytecode modules (`IntelMSR.bin` and `LpcIO.bin`), avoiding the severe security risks of legacy drivers like WinRing0.
- **Embedded & Self-Contained**: The signed PawnIO bytecode modules are embedded directly inside `mutop-ai.exe`. No external module files are needed.
- **Fail-Open Graceful Degradation**: If `PawnIOLib.dll` is not installed or the driver is absent, `mutop-ai` checks once at startup, safely falls back to driverless operation, and incurs zero overhead and zero retries during live monitoring ticks.
- **Hardware Integration Details**:
  - **CPU Package DTS**: Samples MSR `0x1B1` (`IA32_PACKAGE_THERM_STATUS`), computing `TjMax - Delta` (e.g. `100°C - 44 = 56°C`), falling back to `0x19C`.
  - **Carrier Fan RPM**: Under the ISA bus synchronization mutex (`Global\Access_ISABUS.HTP.Method`), accesses the IT8613E Hardware Monitor (`0x0A40`) to read tachometer counter `FAN2` (`CPU_FAN` header on Mu Ultra) at registers `0x0E`/`0x19`, calculating actual RPM (`1,350,000 / (raw * 2)`).
  - **Unprivileged User-Mode Fallback**: When PawnIO is not present, `mutop-ai` displays `CPU: -` and `Fan: Auto` (reflecting autonomous carrier hardware PWM fan curve) and reports NVMe composite temperature.

## CLI Options

```text
USAGE:
    mutop-ai [OPTIONS]

OPTIONS:
    -j, --json               Output a single JSON telemetry snapshot to stdout and exit
    -1, --oneshot            Output a single plaintext telemetry snapshot to stdout and exit
    -s, --stream [MS]        Stream continuous NDJSON snapshots at interval in ms (default: 200ms)
    -w, --watch [SECS]       Stream continuous NDJSON snapshots at interval in seconds (default: 1s)
    -d, --duration <SECS>    Run streaming session for specified duration in seconds and exit
    --summary                Print run summary report (min/median/p90/max) at stream exit
    --active-threshold-pct <PCT> CPU threshold % for tracked process co-activity (default: 50.0)
    --calibrate-idle         Calibrate board idle power baseline at startup from live sensor
    --idle-baseline-w <W>    Override board idle power baseline in Watts (default: 8.5 W)
    --mark-file <PATH>       Inject phase event markers from file into top-level .marker field
    --mcp                    Run headless Model Context Protocol (MCP) JSON-RPC 2.0 server over stdio
    --mcp-port <PORT>        Run headless MCP HTTP + SSE streaming server on 0.0.0.0:<PORT>
    --mcp-listen <ADDR:PORT> Run headless MCP HTTP + SSE streaming server on <ADDR:PORT>
    --schema                 Print comprehensive JSON schema & field dictionary and exit
    -p, --proc <NAMES>       Track specific processes & thread-to-core placement (comma-separated)
    -t, --threads            Include thread-level placement detail in JSON stream
    -v, --version            Print version information and exit
    -h, --help               Print this help message and exit
```

## Interactive TUI Controls

| Key | Action |
|:----|:-------|
| `q`, `Q`, `Esc` | Exit monitor |
| `▲` / `▼` or `j` / `k` | Scroll process table |
| `PgUp` / `PgDn` | Scroll process table by 10 rows |
| `Home` / `g` | Jump to top of process table |
| `c` / `m` / `p` / `n` | Sort processes by CPU / Memory / PID / Name |
| `/` | Filter processes by name |
