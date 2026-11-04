# Cold vs. Warm Startup & Multi-Engine Optimization Research: Intel Core Ultra 7 (Lunar Lake)

**Document ID:** `MUTOP-OPT-RES-01`  
**Target Hardware:** LattePanda Mu Ultra (`DFR1295` Module + `DFR1293` Carrier)  
**Processor:** Intel Core Ultra 7 256V (4× Lion Cove P-cores + 4× Skymont LP-E cores, Intel Arc 140V Xe2-LPG GPU, Intel AI Boost NPU 4)  
**Memory:** 16 GB LPDDR5X-8533 MoP (136.5 GB/s theoretical unified memory bandwidth)  
**Target OS:** Windows 11 Pro 64-bit (Build 10.0.26300 / 24H2)

---

## 1. Executive Overview

This research document analyzes cold vs. warm startup latencies, runtime memory bandwidth optimizations, and Windows 11 hybrid CPU/UMA scheduling dynamics for edge AI workloads running on the LattePanda Mu Ultra. It provides concrete CLI parameters, Win32 API signatures, and architectural strategies, alongside explicit **`[LIMITATION MARKER]`** boundaries that cannot be circumvented in software.

```
+---------------------------------------------------------------------------------------------------------+
|                               INTEL CORE ULTRA 7 256V ARCHITECTURAL STACK                               |
+---------------------------------------------------------------------------------------------------------+
|  Compute:  4x Lion Cove P-Cores (4.8 GHz)  |  4x Skymont LP-E Cores (3.7 GHz)  |  Arc 140V GPU (64 TOPS) |
|            Intel AI Boost NPU 4 (47 TOPS)  |  Intel Thread Director            |  PL1 = 28W Sustained   |
+---------------------------------------------------------------------------------------------------------+
|  Memory:   16 GB LPDDR5X-8533 Memory-on-Package (MoP) | 128-bit Bus | 136.5 GB/s Theoretical Bandwidth   |
|            Windows WDDM / DXCore / MCDM Shared Aperture Limit: 9,156 MB (~57% Physical Capacity)        |
+---------------------------------------------------------------------------------------------------------+
```

---

## 2. GPU SLM Startup & Memory Bandwidth Optimization (`Intel Arc 140V`, `llama.cpp`)

### 2.1 Cold Disk Read vs. Warm OS File Cache (`--mmap`, `--mlock`, `PrefetchVirtualMemory`)

When hosting models such as `Gemma 4 E2B` (~3.1 GB) or `Gemma 4 E4B` (~5.0 GB) on `llama-server.exe`:

1. **`--load-mode mmap` (Default Behavior)**:
   - Calls Win32 `CreateFileMappingW` + `MapViewOfFile`.
   - **Cold Start Behavior:** Process launches almost instantly (<50 ms), as model metadata and tensor headers are mapped into virtual address space without reading all bytes from NVMe storage.
   - **Trade-Off / Latency Spike:** As tensor weights are accessed during the first token prefill, the OS generates a surge of hard/soft page faults. On cold storage, first-token latency (TTFT) degrades by **1.8–4.2 seconds** while pages are faulted from disk.
   - **Warm Start Behavior:** If the file already resides in the Windows System File Cache (standby list), page faults resolve in RAM (<50 ms total load time).

2. **`--load-mode none` (`--no-mmap`)**:
   - Allocates contiguous heap memory via `VirtualAlloc` and reads weights sequentially with `ReadFile`.
   - **Cold Start:** Slower initial server startup (~2.5–3.8 seconds on NVMe SSD at ~1.5 GB/s sequential read).
   - **Advantage:** Completely eliminates runtime page-fault stutter during active token generation.

3. **`--load-mode mmap+mlock` (`--mlock`)**:
   - Calls Win32 `VirtualLock` on the mapped virtual address range.
   - **Advantage:** Pins all model pages into physical RAM, guaranteeing that Windows Memory Manager will never page out or compress model weights during background idle periods or under system memory pressure.
   - **Constraint:** Requires the process to have sufficient working set quota (`SetProcessWorkingSetSizeEx`).

4. **Win32 `PrefetchVirtualMemory` Optimization**:
   ```c
   // Asynchronously primes virtual pages from disk into RAM before inference
   WIN32_MEMORY_RANGE_ENTRY entry;
   entry.VirtualAddress = pMappedModelBuffer;
   entry.NumberOfBytes  = modelFileSizeBytes;
   PrefetchVirtualMemory(GetCurrentProcess(), 1, &entry, 0);
   ```
   - **Best Practice for Device Commander:** Map the GGUF with `mmap`, immediately issue `PrefetchVirtualMemory` in the background during server startup, and execute a 1-token dummy prompt to warm the Vulkan command buffers.

---

### 2.2 Persistent Shader & Kernel Caches (Vulkan vs. SYCL Level Zero)

1. **Vulkan Pipeline Cache (`VkPipelineCache`)**:
   - On the first invocation of `llama-server.exe` (Vulkan backend) with a new model architecture, the Intel Windows Graphics Driver compiles SPIR-V intermediate representation into Intel Gen12/Xe2 native SIMD machine code.
   - **Cold Compilation Overhead:** ~2.1 to 3.5 seconds during the first prefill batch.
   - **Persistent Storage:** Intel driver stores compiled binaries in `%LOCALAPPDATA%\Intel\ShaderCache` and `%LOCALAPPDATA%\D3DSCache`.
   - **Warm Invocations:** Pipeline compilation is bypassed; compiled pipelines load from disk cache in <15 ms.

2. **SYCL Level Zero Persistent JIT Cache (`oneAPI`)**:
   - For `llama.cpp-sycl`, Level Zero JIT compiles OpenCL/SPIR-V kernels to Xe2 hardware binaries.
   - **Environment Variables Required:**
     ```powershell
     $env:SYCL_CACHE_PERSISTENT = "1"
     $env:SYCL_CACHE_DIR = "C:\Users\peter\AppData\Local\Intel\sycl_cache"
     $env:ZE_ENABLE_PCI_ID_DEVICE_ORDER = "1"
     ```
   - If `SYCL_CACHE_PERSISTENT` is unset, `llama-server.exe` recompiles kernels on every cold process launch, adding 8–15 seconds of latency before accepting HTTP queries.

---

### 2.3 Prompt / KV-Cache Pre-Warming & Slot Persistence

In conversational voice assistant turns (Device Commander), the system prompt, tool schemas (FunctionGemma definitions), and conversation rules are fixed.

1. **Persistent Slot Pre-Warming (`--slot-save-path`)**:
   - Parameter: `--slot-save-path C:\develop\mutop\cache\slots`
   - Parameter: `--slot-prompt-similarity 0.10` (or higher)
   - Parameter: `--cache-reuse 64` (minimum chunk size to reuse via KV shifting)
2. **Startup Prompt Evaluation Protocol**:
   - Upon `llama-server.exe` boot, execute an internal dummy query:
     ```json
     {
       "prompt": "<system>You are Device Commander...</system>",
       "n_predict": 1,
       "temperature": 0.0
     }
     ```
   - Pre-fills the entire ~400-token system prompt and persists the KV cache in Slot 0 (`--keep 400`).
   - Subsequent user utterances (e.g. "turn on noise cancelling", 8 tokens) only evaluate the delta user tokens, reducing prefill time from **~400 ms** to **~15 ms** on Arc 140V!

---

### 2.4 UMA Memory Bandwidth Optimization (FlashAttention & Quantized KV Cache)

In unified memory architectures, the primary bottleneck for autoregressive token generation is memory bandwidth ($BW = \text{Model Size} \times \text{Tokens/sec}$).

1. **FlashAttention (`-fa`, `--flash-attn on`)**:
   - Avoids materializing the intermediate $N \times N$ attention matrix in shared RAM by computing softmax in GPU register tiles.
   - On Arc 140V Xe2-LPG, `-fa` reduces prefill memory traffic by **35–45%** on sequences of 512+ tokens, lowering overall SoC power consumption.

2. **Quantized KV Cache (`-ctk` and `-ctv`)**:
   - **Default FP16 (`-ctk f16 -ctv f16`)**: 2 bytes per key/value element. For Gemma 4 E4B with context 2048, the KV cache footprint is ~512 MB.
   - **Quantized INT8 (`-ctk q8_0 -ctv q8_0`)**: 1 byte per element. Halves KV memory bandwidth and cache memory consumption (256 MB) with near-zero perplexity loss.
   - **Quantized INT4 (`-ctk q4_0 -ctv q4_0`)**: 0.5 bytes per element. 75% memory bandwidth reduction.
   - **Impact on UMA Contention:** Reduces GPU memory bus occupancy, allowing simultaneous NPU batch embeddings to proceed without throttling token generation.

---

## 3. STT & TTS Cold vs. Warm Startup & Voice Preloading (`Moonshine` & `Piper`)

### 3.1 ONNX Runtime Offline Graph Optimization Serialization

`onnxruntime` by default optimizes the model computation graph at session initialization:
```python
import onnxruntime as ort

sess_options = ort.SessionOptions()
sess_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
sess_options.optimized_model_filepath = "C:/Users/peter/models/stt/moonshine/base/optimized_encoder.onnx"
# Initial load optimizes and saves graph to disk
session = ort.InferenceSession("model.onnx", sess_options, providers=["CPUExecutionProvider"])
```
- **Cold Initialization (Unoptimized ONNX)**: Runtime performs Constant Folding, LayerNormalization Fusion, Gelu Fusion, and MatMul-Add merging on CPU. Initialization latency: **380–620 ms**.
- **Warm / Pre-Optimized Loading**: Deploying `optimized_encoder.onnx` with `ORT_DISABLE_ALL` bypasses graph transformations completely. Session initialization drops to **22–35 ms**.

---

### 3.2 ONNX Runtime Memory Arena & Thread-Pool Warmup

ONNX Runtime utilizes an internal chunk-based memory arena allocator:
1. `enable_cpu_mem_arena = True`: Replaces standard OS `malloc`/`free` heap allocations with a pre-reserved arena, eliminating thread lock contention on Windows NT heap (`ntdll!RtlAllocateHeap`).
2. `enable_mem_pattern = True`: Pre-computes exact static tensor buffer offsets across model layers.
3. **Execution-Step Page Warmup**:
   - Simply loading the ONNX model does not allocate the actual execution scratchpad tensors.
   - **Warmup Rule:** At service startup, immediately feed a 1-frame dummy audio tensor (`np.zeros((1, 16000), dtype=np.float32)`) through `session.run()`.
   - **Result:** Commits memory arena pages into physical RAM and awakens the OpenMP / ORT worker threads from sleep. Subsequent speech turns execute with zero allocation latency.

---

### 3.3 Piper VITS Neural TTS Optimization: Caching vs. Overlapped Streaming

| Execution Strategy | Latency to Audio Start | Memory Overhead | Description |
|:---|:---:|:---:|:---|
| **Cold Process Spawn (`piper.exe`)** | 1,800 – 2,500 ms | 0 MB (ephemeral) | Spawns process, loads ONNX, parses JSON dictionary, synthesizes. Unacceptable for interactive voice. |
| **Warm Preloaded `InferenceSession`** | 140 – 180 ms | ~180 MB resident | Model and phoneme map kept resident in memory. Synthesizes on demand. |
| **Pre-Synthesized Reflex Phrase Cache (RAM)** | **0 ms (Instant)** | ~12 MB (for 50 phrases) | Deterministic audio buffers (PCM/WAV) stored in RAM. Played immediately upon trigger release. |
| **Chunked Streaming Overlapped TTS** | 60 – 85 ms | ~180 MB resident | First sentence clause synthesized while SLM generates remaining tokens. |

#### Pre-Synthesizing Tier 0 & Tier 1 Phrases
For Device Commander's deterministic catalog (VA-6, VA-7, VA-10):
- Reflex confirmation phrases ("Noise cancelling on", "Microphone muted", "Volume 80%", "HearThrough active") are fixed.
- **Implementation:** Pre-render all 50 catalog confirmations at install/startup to 24 kHz 16-bit mono PCM buffers stored in a memory cache.
- **Latency Outcome:** Audio begins playback within **<10 ms** of command resolution, easily meeting the `VA-7` 1.2s end-to-end confirmation budget.

---

## 4. Windows 11 Hybrid CPU Scheduling & Thread Director Optimization

Intel Core Ultra 7 256V integrates an asymmetric 8-core CPU topology:
- **4× Lion Cove P-Cores (`C0..C3`)**: High IPC, up to 4.8 GHz, 12 MB L3 cache, zero Hyper-Threading.
- **4× Skymont LP-E Cores (`C4..C7`)**: Low-power architectural cluster, up to 3.7 GHz, 4 MB L2 cache, located on the Low Power Island.

```
+-------------------------------------------------------------------------+
|                  CORE ULTRA 7 256V HYBRID CORE TOPOLOGY                 |
+-------------------------------------------------------------------------+
|  Lion Cove P-Cores (C0, C1, C2, C3):   High-Performance Compute Island  |
|  Skymont LP-E Cores (C4, C5, C6, C7):  Low-Power Ambient Island         |
+-------------------------------------------------------------------------+
```

### 4.1 Windows 11 Power Throttling API (EcoQoS vs. HighQoS)

Windows 11 Thread Director dynamically classifies thread workloads based on QoS hints:

```c
#include <windows.h>

// Disable EcoQoS (HighQoS) for Latency-Critical Interactive Voice Threads
PROCESS_POWER_THROTTLING_STATE ThrottleState;
RtlZeroMemory(&ThrottleState, sizeof(ThrottleState));
ThrottleState.Version = PROCESS_POWER_THROTTLING_CURRENT_VERSION;
ThrottleState.ControlMask = PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
ThrottleState.StateMask = 0; // 0 = Disable EcoQoS (forces Lion Cove P-Cores & unthrottled clocks)

SetProcessInformation(
    GetCurrentProcess(),
    ProcessPowerThrottling,
    &ThrottleState,
    sizeof(ThrottleState)
);
```

- **EcoQoS Enabled (`StateMask = PROCESS_POWER_THROTTLING_EXECUTION_SPEED`)**: Windows schedules threads exclusively to Skymont LP-E cores and lowers uncore power rails. Suitable for background telemetry collectors (`mutop-ai`), log forwarders, and idle listeners.
- **EcoQoS Disabled (`StateMask = 0`)**: Windows scheduler prioritizes Lion Cove P-cores (`C0..C3`) for burst prompt processing and low-latency endpointing.

---

### 4.2 OpenVINO & ORT Thread Affinity Placement

In Wave 2 and Wave 3 benchmarks, dedicated tests verified thread execution behavior:
1. **OpenVINO Core Affinity:**
   - `ov::hint::scheduling_core_type` can be explicitly set to:
     - `ov::hint::SchedulingCoreType::PCORE_ONLY`: Restricts inference to `C0..C3`.
     - `ov::hint::SchedulingCoreType::ECORE_ONLY`: Restricts inference to `C4..C7`.
2. **Audio Island Validation:**
   - `Moonshine Tiny` STT executes in **87.1 ms** (RTF 0.0087) on LP-E cores at only **5.93 W** board power.
   - `Piper` TTS synthesizes 20 phrases in **2.91 s** (RTF 0.0705) on LP-E cores at **3.70 W** board power (saving 12.9% power vs P-cores).
   - **Architectural Takeaway:** Audio pipelines (STT & TTS) should be pinned to Skymont LP-E cores, preserving 100% of the Lion Cove P-core cluster for OS tasks and UI interactivity.

---

## 5. Hardware-Enforced Limitations (`[LIMITATION MARKER]`)

The following physical and architectural boundaries are enforced by silicon and board design on the LattePanda Mu Ultra and **cannot be eliminated via software optimizations**:

### `[LIMITATION MARKER: UMA_MEMORY_BANDWIDTH_136GBPS]`
* **Hardware Ceiling:** Dual-channel 128-bit LPDDR5X-8533 Memory-on-Package provides a maximum theoretical bandwidth of:
  $$\text{Bandwidth} = 8533 \times 10^6 \times 16 \text{ bytes} \approx 136.5 \text{ GB/s}$$
* **Constraint:** This memory bus is shared simultaneously between CPU, Arc 140V GPU, and Intel AI Boost NPU.
* **Impact:** During Arc 140V token decode on a 5 GB model generating at ~27 tok/s, GPU memory traffic approaches $\sim 135\text{ GB/s}$, saturating the memory bus. Concurrent memory-intensive NPU batch embeddings or heavy CPU RAM copies will contend on this bus, lowering GPU decode throughput by 10–25%.

### `[LIMITATION MARKER: PHYSICAL_MOP_CAPACITY_16GB]`
* **Hardware Ceiling:** 16 GB LPDDR5X is integrated directly onto the Lunar Lake compute package (Memory-on-Package). It cannot be expanded or upgraded.
* **Constraint:** Usable system memory exposed to Windows 11 is ~15.6 GB.

### `[LIMITATION MARKER: DXCORE_MCDM_SHARED_APERTURE_9156MB]`
* **Hardware Ceiling:** Windows WDDM 3.2 allocates a maximum shared aperture of **9,156 MB** (~57% of physical RAM) across DirectX/DXCore and MCDM compute devices.
* **Constraint:** The sum of:
  $$\text{VRAM}_{\text{GPU}} + \text{VRAM}_{\text{NPU}} \le 9,156\text{ MB}$$
* **Impact:** Attempting to simultaneously hold `Gemma 4 E4B` (5.0 GB) + `Whisper Medium` (1.5 GB) + `EmbeddingGemma` (0.7 GB) + `FunctionGemma` (0.4 GB) alongside large KV caches will exceed this aperture, causing OS allocation failure (`E_OUTOFMEMORY`) or severe page eviction thrashing.

### `[LIMITATION MARKER: CARRIER_PL1_POWER_28W]`
* **Hardware Ceiling:** The LattePanda Mu carrier board BIOS and 5V PWM blower fan enforce a long-duration power limit (PL1) of **28 W** (with short-duration PL2 boost capped at ~37 W).
* **Constraint:** Simultaneous saturation of Arc 140V GPU (peak 20 W), NPU 4 (peak 4.5 W), and CPU cores (peak 15 W) would demand >39.5 W, which triggers RAPL package throttling.
* **Impact:** Under sustained tri-accelerator stress (Scenario 2B), the hardware RAPL controller dynamically throttles GPU core clocks (down to ~1.4–1.6 GHz) and CPU clocks to enforce the 28W ceiling, as verified in telemetry (max package power reached 26.46 W).

### `[LIMITATION MARKER: NPU_STATIC_SHAPE_COMPILATION]`
* **Hardware Ceiling:** Intel AI Boost NPU 4 and its MCDM driver require static tensor shapes defined at compilation time.
* **Constraint:** Arbitrary dynamic sequence lengths cannot be executed on NPU without triggering full JIT graph re-compilation (costing 1.5–5.0 seconds of latency) or silently falling back to the CPU execution provider.
* **Workaround:** Models on NPU must be pre-compiled with discrete shape buckets (e.g. `[1, 64]`, `[1, 128]`, `[4, 512]`).

---

## 6. Synthesis: Recommended Execution Configuration for Device Commander

| Component | Target Accelerator | Load & Cache Policy | Thread & QoS Configuration | Expected Latency |
|:---|:---|:---|:---|:---:|
| **Reflex Audio (Earcons & TTS)** | CPU (Audio Island) | Pre-rendered RAM buffer | Win32 Audio Output | **< 10 ms** |
| **S1 Grammar / Reflex NLU** | CPU (Skymont LP-E) | Pre-compiled Vosk / Lexicon | EcoQoS (`C4..C7`) | **< 25 ms** |
| **S3 Intent (FunctionGemma)** | Intel AI Boost NPU 4 | OpenVINO MCDM (Static INT8) | MCDM Driver Queue | **~35 – 50 ms** |
| **Semantic Embeddings** | Intel AI Boost NPU 4 | Static shape buckets `[1, 128]` | MCDM Driver Queue | **~10 ms** |
| **Speech-to-Text (Moonshine)** | CPU (Skymont LP-E) | Pre-optimized ONNX + Warm Arena | ORT `ECORE_ONLY` (4 threads) | **~85 ms** (RTF 0.009) |
| **Dynamic SLM (Gemma 4 E2B)** | Intel Arc 140V GPU | Vulkan + FlashAttn + Slot Warmup | HighQoS / GPU Queue | **~25 – 38 tok/s** |
| **Dynamic TTS (Piper VITS)** | CPU (Skymont LP-E) | Persistent Session + Streaming | ORT `ECORE_ONLY` (4 threads) | **~145 ms** |
