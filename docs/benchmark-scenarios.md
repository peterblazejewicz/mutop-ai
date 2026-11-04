# Multi-Engine AI Verification & Concurrent Workflow Benchmark Suite

**Target Platform:** LattePanda Mu Ultra (`DFR1295` Compute Module + `DFR1293` Mini Carrier Board)  
**Host Processor:** Intel Core Ultra 7 256V ("Lunar Lake")  
**Compute Engines:**
- **CPU:** 8 Cores / 8 Threads (4× Lion Cove P-cores `C0..C3` @ 4.8 GHz + 4× Skymont LP-E cores `C4..C7` @ 3.7 GHz, zero SMT)
- **GPU:** Intel Arc 140V (8 Xe2-LPG Battlemage cores, 64 TOPS INT8, 1.95 GHz, 9,156 MB shared aperture)
- **NPU:** Intel AI Boost (NPU 4, PCI `0x8086:0x643E`, 47 TOPS INT8, MCDM driver)
- **Memory:** 16 GB LPDDR5X-8533 Memory-on-Package (MoP, 136.5 GB/s unified memory architecture)
- **Power & Cooling:** Intel RAPL (`PKG`, `PP0`, `PP1`, `DRAM`), 5V PWM Active Fan (`CPU_FAN`, up to 5,300 RPM)
- **Primary OS:** Windows 11 Pro 64-bit (`10.0.26300` / 24H2)

---

## 1. Scope & Telemetry Objectives

This specification defines the multi-engine AI verification scenarios and concurrent workload benchmarks designed to evaluate real-time hardware telemetry in `mutop-ai` under realistic edge AI operations.

### 1.1 Core Telemetry Verification Goals

1. **Hardware Engine Isolation & Non-Double-Counting**:
   - Verify orthogonal attribution of shared UMA memory across `pure_cpu_in_use_mb`, `arc_gpu_shared_mb`, and `npu_shared_mb` without exceeding physical MoP boundaries or double-counting host working sets.
   - Measure engine-specific utilization across DXCore MCDM NPU compute nodes (`npu.utilization_pct`), D3DKMT/PDH GPU compute engines (`gpu.compute_pct`, `gpu.total_utilization_pct`), and per-core CPU microarchitectures (`cpu.lion_cove_p_load_pct` vs. `cpu.skymont_lpe_load_pct`).
2. **Energy & Power Rail Attribution**:
   - Verify unprivileged Windows Energy Metering Interface (EMI) and RAPL rails:
     $$P_{\text{PKG}} \approx P_{\text{PP0 (CPU)}} + P_{\text{PP1 (GPU/Uncore)}} + P_{\text{DRAM}} + P_{\text{SoC Baseline}}$$
   - Quantify total energy consumption per inference turn and real-time factor:
     $$E = \int_{t_{\text{start}}}^{t_{\text{end}}} P_{\text{PKG}}(t)\,dt \approx \sum_{i=1}^{N} P_{\text{PKG}}(t_i) \cdot \Delta t_i \quad [\text{Joules}]$$
3. **Thermal & Fan Dynamics**:
   - Monitor live CPU Package DTS (`IA32_PACKAGE_THERM_STATUS` MSR `0x1B1` via PawnIO) and carrier blower tachometer (`CPU_FAN` RPM via ITE IT8613E Super I/O) as active cooling ramps from nominal (~2200 RPM) under sustained PL1/PL2 workloads.
4. **Thread Director & Core Placement**:
   - Observe worker thread scheduling across Lion Cove P-cores (`0..3`) vs. Skymont LP-E cores (`4..7`), verifying whether background EcoQoS policies or scheduler heuristics displace latency-critical AI threads.
5. **AI Diagnostics Rule Engine**:
   - Validate live triggering and recovery of:
     - `llm_worker_on_skymont_lpe_cores`
     - `uma_memory_pressure_near_budget` (shared memory exceeding 85% of 9,156 MB aperture)
     - `gpu_or_cpu_power_throttling` (`mhz_limit < max_mhz` or GPU limiter flags)
     - `timer_resolution_coarse_during_voice_turn` (timer tick > 1.0 ms while audio pipeline active)

---

## 2. Phase 1: Individual Component Baselines (Single-Engine Isolation)

Phase 1 benchmarks establish isolated compute, memory, power, and thermal baselines for each execution engine on the LattePanda Mu Ultra.

```
+----------------------------------------------------------------------------------------------------+
|                                PHASE 1: ISOLATED BASELINES                                         |
+----------------------------------------------------------------------------------------------------+
|  [Scenario 1A] NPU FunctionGemma-270M (Tool-Call Intent Classifier)                                |
|  [Scenario 1B] NPU EmbeddingGemma-300M (Semantic Vector Embeddings)                                |
|  [Scenario 1C] GPU Gemma 4 E2B & E4B (Prefill PP512 + Decode TG128)                                |
|  [Scenario 1D] Tri-Engine Whisper Comparison (base / small / medium / turbo on NPU vs GPU vs CPU)  |
|  [Scenario 1E] CPU/GPU Moonshine STT (tiny / base / medium-streaming-en)                           |
|  [Scenario 1F] CPU Piper Neural TTS (Phoneme Synthesis & P-Core vs LP-E Placement)                 |
+----------------------------------------------------------------------------------------------------+
```

### Scenario 1A: NPU Tool-Call Intent Classification (`FunctionGemma-270M`)

* **Model:** `FunctionGemma-270M` (INT8 / Q8_0, static shape compilation).
* **Target Engine:** Intel AI Boost NPU 4 (MCDM compute driver, OpenVINO / DirectML / ONNX Runtime).
* **Payload:** 100 consecutive structured device configuration turns (e.g. volume adjustment, ANC toggles, audio mode selections).
* **Execution Parameters:** Batch size = 1, fixed input sequence length = 64 tokens, max output tokens = 32.
* **Telemetry Verification Focus:**
  - `npu.utilization_pct`: Node compute active percentage during inference bursts.
  - `npu.shared_memory_used_mb`: NPU static buffer footprint (expected: 320–480 MB).
  - `power_thermals.package_w`: Low-power envelope (expected: < 4.5 W package power during NPU execution).
  - `power_thermals.core_pp0_w` & `power_thermals.gpu_pp1_w`: Near-idle readings (< 1.0 W CPU, 0.0 W GPU).
  - `ai_diagnostics.llm_worker_on_skymont_lpe_cores`: Must remain `false` (orchestration on host, execution on NPU).

### Scenario 1B: NPU Semantic Embeddings (`EmbeddingGemma-300M`)

* **Model:** `EmbeddingGemma-300M` (FP16 / INT8, static sequence shapes: 128 and 512 tokens).
* **Target Engine:** Intel AI Boost NPU 4.
* **Payload:** 50 document chunks embedded individually and in micro-batches of 4.
* **Telemetry Verification Focus:**
  - `npu.utilization_pct`: Continuous sustained NPU duty cycle during batch processing.
  - Embedding throughput: Vectors per second ($\text{vec/s}$) and latency per 512-token vector.
  - Energy per embedding:
    $$E_{\text{vec}} = \frac{E_{\text{total}}}{N_{\text{vectors}}} \quad [\text{Joules/vector}]$$
  - Verification that DRAM power (`power_thermals.dram_w`) reflects LPDDR5X memory bandwidth during weights streaming.

### Scenario 1C: GPU Small Language Model (`Gemma 4 E2B` & `Gemma 4 E4B`)

* **Model Variants:**
  1. `Gemma 4 E2B` (~2B parameters, Q4_K_M / Q8_0, ~1.5–2.2 GB VRAM).
  2. `Gemma 4 E4B` (~4B parameters, Q4_K_M, ~2.8–3.5 GB VRAM).
* **Target Engine:** Intel Arc 140V GPU (Vulkan / SYCL backend via `llama-server.exe`).
* **Workload Phases:**
  1. **Prefill Phase (PP512):** Prompt processing of 512 prompt tokens.
  2. **Decode Phase (TG128):** Autoregressive generation of 128 tokens.
* **Telemetry Verification Focus:**
  - `gpu.compute_pct`: Compute shader utilization during PP512 and TG128.
  - `gpu.shared_memory_used_mb`: Shared memory reservation matching model weights + KV cache footprint.
  - `power_thermals.gpu_pp1_w`: RAPL GPU rail activation (expected: 8–18 W during prefill bursts).
  - `llama_server` slots telemetry (`loopback.llama_server_listening`, `/slots` tok/s).
  - Throttling indicators: `gpu.throttling.power_limited` under PL1/PL2 transitions.

### Scenario 1D: Whisper STT Tri-Engine Comparison (NPU vs. GPU vs. CPU)

Evaluates speech recognition performance, power, and efficiency across all four major Whisper model tiers and all three compute engines.

#### Model Tiers Evaluated:
1. `whisper-base` (~74M parameters, lightweight baseline)
2. `whisper-small` (~244M parameters, balanced accuracy)
3. `whisper-medium` (~769M parameters, 24 encoder + 24 decoder layers)
4. `whisper-large-v3-turbo` (~809M parameters, 32 encoder + 4 decoder layers)

#### Architectural Contrast Note:
- `whisper-medium` features a symmetric deep decoder (24 encoder layers, 24 decoder layers), incurring high autoregressive decoding overhead.
- `whisper-large-v3-turbo` features an asymmetric architecture (32 encoder layers, 4 decoder layers), shifting computational weight to parallel encoder execution while substantially speeding up autoregressive decoding.
- This contrast directly exercises compiler optimization, static vs. dynamic shape handling, and memory bandwidth across accelerators:
  - **NPU:** Static shape compilation overhead, matrix tile utilization, and autoregressive token latency.
  - **GPU:** Xe2 matrix compute (XMX) throughput and shared memory bandwidth.
  - **CPU:** AVX2/VNNI SIMD scaling and cache hierarchy differences between Lion Cove P-cores and Skymont LP-E cores.

#### Target Execution Engines:
| Engine ID | Device Target | Runtime / Provider | Thread / Affinity Configuration |
| :--- | :--- | :--- | :--- |
| **`NPU`** | Intel AI Boost NPU 4 | OpenVINO / DirectML NPU EP | 1 execution stream, static audio shapes |
| **`GPU`** | Intel Arc 140V (Xe2-LPG) | Vulkan / SYCL / OpenVINO GPU EP | Dynamic batching, compute shader pipeline |
| **`CPU-P`** | Lion Cove P-cores (`C0..C3`) | OpenVINO CPU / whisper.cpp AVX2 | Process Affinity mask `0x0F` (cores 0..3) |
| **`CPU-LPE`**| Skymont LP-E cores (`C4..C7`) | OpenVINO CPU / whisper.cpp AVX2 | Process Affinity mask `0xF0` (cores 4..7) |

#### Standardized Payloads:
- **Utterance A:** 10.0-second speech sample (standard command and dialogue).
- **Utterance B:** 30.0-second continuous speech sample (dense meeting transcript).

#### Comparative Metrics Computed by `mutop-ai`:
1. **Real-Time Factor (RTF)**:
   $$\text{RTF} = \frac{\Delta t_{\text{inference}} [s]}{T_{\text{audio}} [s]}$$
   (RTF < 1.0 indicates faster than real time; lower is better).
2. **Energy Consumed per Utterance ($E_{\text{utterance}}$)**:
   $$E_{\text{utterance}} = \int_{0}^{\Delta t_{\text{inference}}} P_{\text{PKG}}(t)\,dt \quad [\text{Joules}]$$
3. **Normalized Energy per Audio Second ($J/s$)**:
   $$\text{Energy Rate} = \frac{E_{\text{utterance}}}{T_{\text{audio}}} \quad [\text{Joules / audio-second}]$$
4. **Engine Attribution**:
   - On `CPU-P`: `cpu.lion_cove_p_load_pct` > 70%, `cpu.skymont_lpe_load_pct` ~ 0%.
   - On `CPU-LPE`: `cpu.skymont_lpe_load_pct` > 70%, `cpu.lion_cove_p_load_pct` ~ 0%.
   - On `GPU`: `gpu.compute_pct` > 50%, `power_thermals.gpu_pp1_w` > 5.0 W.
   - On `NPU`: `npu.utilization_pct` > 50%, `power_thermals.core_pp0_w` < 1.5 W.

### Scenario 1E: Moonshine STT (`tiny`, `base`, `medium-streaming-en`)

* **Model Variants:**
  1. `moonshine-tiny` (~27M parameters, ultra-low latency).
  2. `moonshine-base` (~61M parameters).
  3. `moonshine-medium-streaming-en` (variable-length streaming encoder).
* **Target Engine:** CPU (with optional GPU DirectML fallback).
* **Workload:** 10-second streaming speech chunks processed incrementally with VAD boundaries.
* **Telemetry Verification Focus:**
  - Streaming latency per audio chunk (chunk processing time vs. chunk duration).
  - Skymont LP-E core utilization: Evaluating whether lightweight streaming STT can run entirely on low-power Skymont cores without causing audio stutter.

### Scenario 1F: Piper Neural TTS

* **Model:** Piper ONNX voice model (`en_GB-southern_english_female-medium`).
* **Target Engine:** CPU (ONNX Runtime / Piper native).
* **Payload:** 3 synthetic responses: short earcon confirmation (5 words), standard setting confirmation (15 words), detailed diagnostic summary (45 words).
* **Telemetry Verification Focus:**
  - Phoneme synthesis RTF ($\Delta t_{\text{synthesis}} / T_{\text{audio}}$).
  - Thread affinity verification: Observing whether the TTS synthesizer executes on Lion Cove P-cores (`0..3`) or Skymont LP-E cores (`4..7`) under default Windows 11 Thread Director heuristics.

---

## 3. Phase 2: Simultaneous Multi-Engine Composite Workloads

Phase 2 simulates a realistic next-generation edge AI copilot running concurrently on a mid-range workforce laptop (represented by the LattePanda Mu Ultra).

```
+----------------------------------------------------------------------------------------------------+
|                                PHASE 2: CONCURRENT WORKLOADS                                       |
+----------------------------------------------------------------------------------------------------+
|  [Scenario 2A] Concurrent Voice Assistant Turn:                                                    |
|    - Engine 1 (CPU): Audio capture + STT (Moonshine/Whisper) + TTS (Piper)                         |
|    - Engine 2 (NPU): Intent classification (FunctionGemma) + RAG retrieval (EmbeddingGemma)       |
|    - Engine 3 (GPU): Local reasoning / answer generation (Gemma 4 E2B/E4B SLM)                     |
|                                                                                                    |
|  [Scenario 2B] Sustained Tri-Accelerator Stress & UMA Contention (60s):                            |
|    - Simultaneous NPU (Intent + Embeddings) + GPU (Gemma 4 E4B) + CPU (Whisper + Piper)            |
|    - Telemetry: UMA split reconciliation, RAPL rail bounds, thermal dissipation & fan RPM curve    |
+----------------------------------------------------------------------------------------------------+
```

### Scenario 2A: Full Voice Turn Cycle (Tri-Engine Concurrency)

This scenario models the end-to-end voice assistant interaction loop while background SLM generation is active:

1. **Step 1 (Audio STT on CPU / Engine 1)**: User speech (5s audio) transcribed via `Moonshine` or `Whisper-base`.
2. **Step 2 (NPU Intent & Vector Retrieval / Engine 2)**:
   - Tool intent classified by `FunctionGemma-270M` on Intel AI Boost NPU.
   - Context embedding generated by `EmbeddingGemma-300M` on NPU.
3. **Step 3 (GPU SLM Generation / Engine 3)**:
   - `Gemma 4 E2B` generates a contextual conversational answer on Intel Arc 140V GPU.
4. **Step 4 (CPU TTS / Engine 1)**:
   - Synthesized voice stream emitted via `Piper` on CPU.

#### Telemetry Verification Checklist:
- [ ] Active accelerator share reflects co-activity (`Active NPU & GPU Share > 0%`).
- [ ] Loopback connection detection: `loopback.active_loopback_connections` tracks local HTTP/JSON-RPC exchanges.
- [ ] Zero audio underruns: Invariant TSC and timer resolution stay bounded (`timing.timer_resolution.current_ms <= 1.0`).

### Scenario 2B: Sustained Tri-Accelerator Stress & UMA Contention Test (60 Seconds)

Sustained maximum-concurrency test running all three compute engines at peak load for 60 seconds:
- **NPU:** Continuous batch vector embeddings (`EmbeddingGemma`) + looped tool evaluations (`FunctionGemma`).
- **GPU:** Continuous token generation loop (`Gemma 4 E4B`, prompt prefill 512 + decode 256 looped).
- **CPU:** Continuous STT transcription (`Whisper-small`) + Piper TTS synthesis on 8 threads.

#### Critical Telemetry Invariants to Verify:

#### A. Unified Memory Architecture (UMA) Non-Double-Counting
On Lunar Lake, the 16,384 MB MoP is a shared physical pool:
- $\text{Total Installed MoP} = 16,384\text{ MB}$
- $\text{BIOS Aperture Stolen} \approx 320\text{ MB}$
- $\text{OS Total Physical} \approx 16,064\text{ MB}$

`mutop-ai` must report:
$$\text{In-Use Physical RAM} = \text{Pure CPU In-Use} + \text{Arc Shared Used} + \text{NPU Shared Used}$$
$$\text{Shared Memory Total} = (\text{Arc Shared Used} + \text{NPU Shared Used}) \le 9,156\text{ MB (Shared Aperture)}$$
- Verify that `memory.in_use_phys_mb` accurately tracks the true sum without double-counting GPU/NPU pinned host allocations in `pure_cpu_in_use_mb`.

#### B. RAPL Power Distribution
- Verify that Package power equals the sum of component rails plus uncore baseline:
  $$P_{\text{PKG}} \ge P_{\text{PP0 (CPU)}} + P_{\text{PP1 (GPU)}} + P_{\text{DRAM}}$$
- Under peak tri-accelerator load, verify that Package power reaches PL1 sustained rating (~28–30 W) or PL2 peak turbo (~35–37 W) and throttles cleanly.

#### C. Thermal Ramp & Carrier Fan Speed
- Monitor CPU Package DTS temperature (`power_thermals.cpu_package_dts_c`) climbing from baseline (~40°C) toward 75°C–85°C.
- Verify that the carrier board `CPU_FAN` tachometer (`power_thermals.cpu_fan_rpm`) responds dynamically, ramping up from ~2,200 RPM up toward 4,500–5,300 RPM.

#### D. Thread Director Placement & Core Contention
- Monitor `cpu.lion_cove_p_load_pct` vs. `cpu.skymont_lpe_load_pct`.
- Verify that `mutop-ai`'s process tracker correctly identifies whether GPU/NPU orchestration threads remain on Lion Cove P-cores or get demoted to Skymont LP-E cores under heavy system load.

#### E. AI Diagnostics Rules Activation
Verify the rule engine correctly trips under contention:
- `uma_memory_pressure_near_budget`: Trips if shared memory allocation exceeds 85% of 9,156 MB (~7,780 MB).
- `gpu_or_cpu_power_throttling`: Trips when `mhz_limit < max_mhz` or `power_limited == true`.
- `llm_worker_on_skymont_lpe_cores`: Trips if worker threads for `llama-server.exe` run on cores 4..7.

---

## 4. Telemetry Field Verification Matrix

| Telemetry Field in `MutopSnapshot` | Source API on Windows 11 | Phase 1 Isolated Target | Phase 2 Concurrent Target | Invariant / Boundary Condition |
| :--- | :--- | :--- | :--- | :--- |
| `npu.utilization_pct` | MCDM / DXCore / D3DKMT | 40–95% (Scen 1A, 1B, 1D-NPU) | 30–80% | 0.0% when idle; power-gated |
| `npu.shared_memory_used_mb` | DXCore adapter memory | 300–600 MB | 400–800 MB | Cannot exceed shared aperture |
| `gpu.total_utilization_pct` | D3DKMT / PDH GPU Engine | 60–98% (Scen 1C, 1D-GPU) | 70–100% | 0.0% when idle |
| `gpu.compute_pct` | D3DKMT Compute Engine | 60–98% (Gemma / Whisper) | 60–95% | Differentiates from 3D/Render |
| `gpu.shared_memory_used_mb` | DXGI `QueryVideoMemoryInfo` | 1,800–3,800 MB | 2,500–4,500 MB | Bounded by 9,156 MB aperture |
| `cpu.lion_cove_p_load_pct` | `NtQuerySystemInformation` | 0–100% (isolated to C0..C3) | 80–100% | 4 cores @ up to 4.8 GHz |
| `cpu.skymont_lpe_load_pct` | `NtQuerySystemInformation` | 0–100% (isolated to C4..C7) | 40–90% | 4 cores @ up to 3.7 GHz |
| `memory.pure_cpu_in_use_mb` | Reconciled GlobalMemory | 2,000–3,200 MB | 2,500–4,000 MB | $\text{InUse} - (\text{GPU} + \text{NPU})$ |
| `power_thermals.package_w` | RAPL PKG via EMI / PDH | 3.5–18.0 W | 25.0–37.0 W | Bounded by PL1 (30W) / PL2 (37W)|
| `power_thermals.core_pp0_w` | RAPL PP0 (CPU cores) | 1.0–15.0 W | 10.0–20.0 W | $\le P_{\text{PKG}}$ |
| `power_thermals.gpu_pp1_w` | RAPL PP1 (Arc GPU) | 0.0–16.0 W | 8.0–16.0 W | $\le P_{\text{PKG}}$ |
| `power_thermals.dram_w` | RAPL DRAM (MoP) | 0.05–1.2 W | 0.8–2.5 W | Increases with memory bandwidth |
| `power_thermals.cpu_package_dts_c` | MSR `0x1B1` via PawnIO | 40.0–65.0 °C | 65.0–88.0 °C | Max junction TjMax = 100°C |
| `power_thermals.cpu_fan_rpm` | ITE IT8613E via PawnIO | 2,200–2,800 RPM | 3,000–5,300 RPM | 5V PWM active fan response |
| `timing.timer_resolution` | `NtQueryTimerResolution` | 0.5–1.0 ms (active) | 0.5–1.0 ms | Must not drift to 15.6 ms |

---

## 5. Benchmark Orchestration & Phase Marker IPC Protocol

To capture precise statistical interval summaries for each scenario without modifying AI engine internals, `mutop-ai` supports external phase marker injection via `--mark-file <path>`.

### 5.1 Marker Protocol Format

In `edgetop-core` and `mutop-ai`, `consume_mark_file` reads plain-text lines from `--mark-file` and immediately truncates the file. Consecutive marker tokens form labeled intervals in the summary report:
$$\text{Interval}_i = \text{marker}_i \to \text{marker}_{i+1}$$

To guarantee that `mutop-ai -s 200` samples each transition without race conditions:
1. The marker writer writes a single plain-text token (e.g. `1A_npu_warmup\n`) by truncating/overwriting the file.
2. The marker writer pauses for at least $\Delta t \ge 0.25\text{ s}$ to guarantee that `mutop-ai` samples the transition on its 200 ms polling tick.
3. Terminal markers (e.g. `1A_done`, `done`, `STREAM_END`) close the final interval.

#### Example Sequence for Scenario 1A (`NPU FunctionGemma`):
```text
1A_npu_warmup
1A_npu_functiongemma_infer
1A_done
```

#### Example Sequence for Scenario 1D (`Whisper Tri-Engine`):
```text
1D_whisper_base_npu
1D_whisper_base_gpu
1D_whisper_base_cpu_p
1D_whisper_base_cpu_lpe
1D_whisper_small_npu
1D_whisper_small_gpu
1D_whisper_small_cpu_p
1D_whisper_small_cpu_lpe
1D_whisper_medium_npu
1D_whisper_medium_gpu
1D_whisper_medium_cpu_p
1D_whisper_medium_cpu_lpe
1D_whisper_turbo_npu
1D_whisper_turbo_gpu
1D_whisper_turbo_cpu_p
1D_whisper_turbo_cpu_lpe
1D_done
```

### 5.2 Summary Generation Output

When `--summary` is passed, `mutop-ai` computes and prints statistical distributions (Min, Median, P90, Max) for each interval marker transition:

```text
================================================================================
                       MUTOP-AI RUN SUMMARY REPORT
================================================================================
Duration / Samples : 120 samples (~24.0s)
Active NPU & GPU Share: 78.3% (94/120 samples)

METRIC                              MIN     MEDIAN        P90        MAX
--------------------------------------------------------------------------------
Board Power (W)                    4.12      26.40      34.80      36.50
Fan Speed (RPM)                    2220       3450       4820       5180
CPU Total Load (%)                  4.2       45.0       78.2       88.4
  Big Cores Load (%)                0.0       68.0       95.0       98.0
  Little Cores Load (%)             0.0       22.0       61.0       78.0
GPU Active Duty (%)                 0.0       72.5       94.0       98.0
RAM Available (GB)                 8.40       9.20      11.50      12.60
Thermals:
  cpu (C)                          42.0       68.0       81.0       84.0
  nvme (C)                         30.0       32.0       34.0       34.0

--------------------------------------------------------------------------------
MARKER PHASES & INTERVAL PROFILING (16 intervals)
--------------------------------------------------------------------------------
[01] Phase : 1D_whisper_base_npu -> 1D_whisper_base_gpu
     Samples: 15 (3.0s) | NPU 78.4% | GPU 0.0% | CPU 6.2% | Fan 2220 RPM
     Power  : med 5.82 W / max 7.10 W | CPU Temp: med 44.0 C / max 47.0 C
[02] Phase : 1D_whisper_base_gpu -> 1D_whisper_base_cpu_p
     Samples: 12 (2.4s) | NPU 0.0% | GPU 84.2% | CPU 8.4% | Fan 2350 RPM
     Power  : med 14.20 W / max 18.50 W | CPU Temp: med 52.0 C / max 56.0 C
[03] Phase : 1D_whisper_base_cpu_p -> 1D_whisper_base_cpu_lpe
     Samples: 18 (3.6s) | NPU 0.0% | GPU 0.0% | CPU 48.0% (P 92.0% / LPE 4.0%)
     Power  : med 16.80 W / max 22.40 W | CPU Temp: med 58.0 C / max 63.0 C
[04] Phase : 1D_whisper_base_cpu_lpe -> 1D_whisper_small_npu
     Samples: 35 (7.0s) | NPU 0.0% | GPU 0.0% | CPU 36.0% (P 2.0% / LPE 70.0%)
     Power  : med 9.40 W / max 12.10 W | CPU Temp: med 49.0 C / max 51.0 C
...
```

---

## 6. Execution Safety & Board Occupancy Policy

> [!CAUTION]
> **SAFETY GATE: DO NOT RUN BENCHMARK WORKLOADS ON `mu` WITHOUT EXPLICIT USER APPROVAL.**
> - The LattePanda Mu Ultra board is a shared target host.
> - Before executing any run on `mu`, the human operator must be informed to confirm board availability with peer agents.
> - All benchmark runner scripts must provide a `--dry-run` validation mode that tests marker generation, IPC file handling, and argument validation locally without starting remote processes.
