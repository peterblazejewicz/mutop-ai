# mutop-ai

High-performance hardware telemetry and AI monitor for the **LattePanda Mu Ultra** (DFR1295 compute module with Intel Core Ultra 7 256V Lunar Lake SoC + DFR1293 Mini Carrier Board).

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
| **Tier 3: Signed Driver** | Kernel driver / Ring 0 | Direct Model-Specific Registers (MSRs) for RAPL energy counters (`0x606`, `0x611`, `0x659`), digital thermal sensors (DTS) | Falls back gracefully to Windows Energy Meter PDH counters |

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
