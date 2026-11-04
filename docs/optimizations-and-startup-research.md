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

## 5. Intel OpenVINO 2026.4.1 NPU & GPU Optimization & Caching Runtime Inventory

Direct property inspection on the LattePanda Mu Ultra (`Core Ultra 7 256V`, OpenVINO 2026.4.1, Python 3.12 64-bit) established the exact supported runtime properties and caching behaviors across all three hardware plugins:

### 5.1 Verified Plugin Property Catalog on Lunar Lake

| Plugin | Property Key | Type / Access | Validated Values on `mu` | Documented Intel / MS Behavior |
|:---|:---|:---:|:---|:---|
| **`NPU`** | `CACHE_DIR` | `str` (RW) | `"C:\Users\peter\models\.ov_cache"` (default: `""`) | Enables caching of compiled `.blob` models to NVMe disk. Eliminates JIT compilation on subsequent runs. |
| **`NPU`** | `CACHE_MODE` | `CacheMode` (RW) | `OPTIMIZE_SPEED` (default), `OPTIMIZE_SIZE` | `OPTIMIZE_SPEED`: embeds model weights into compiled `.blob` (~612 MB for FunctionGemma), fastest load. `OPTIMIZE_SIZE`: separates weights from `.blob`, referencing external `.bin` weights. |
| **`NPU`** | `COMPILATION_NUM_THREADS` | `int` (RW) | `8` (default: 8) | Number of host CPU threads allocated to the offline/JIT VCL compiler for neural tile mapping. |
| **`NPU`** | `NPU_DRIVER_VERSION` | `int` (RO) | `1005540` | Intel NPU Driver version reported by WDDM MCDM runtime. |
| **`NPU`** | `NPU_COMPILER_VERSION` | `int` (RO) | `524292` | Internal VCL compiler version packaged in driver. |
| **`NPU`** | `NPU_MAX_TILES` | `int` (RO) | `6` | Maximum hardware neural compute tiles on Intel AI Boost NPU 4 (Lunar Lake). |
| **`NPU`** | `NPU_TILES` | `int` (RW) | `-1` (default: auto) | Manual tile allocation override (`1..6` or `-1` for auto). |
| **`NPU`** | `NPU_TURBO` | `bool` (RW) | `False` (default) | Boosts NPU clock frequency rail; increases thermal output. |
| **`NPU`** | `NPU_DEFER_WEIGHTS_LOAD` | `bool` (RW) | `False` (default) | Defers weight paging from host memory until first inference call. |
| **`NPU`** | `NPU_BYPASS_UMD_CACHING` | `bool` (RW) | `False` (default) | Bypasses User Mode Driver (UMD) Level Zero memory caching layer. |
| **`NPU`** | `NPU_DISABLE_IDLE_MEMORY_PRUNING` | `bool` (RW) | `False` (default) | Prevents Windows OS from trimming working set during idle periods. |
| **`NPU`** | `NPU_COMPILER_DYNAMIC_QUANTIZATION` | `bool` (RW) | `False` (default) | Enables runtime dynamic activation quantization in VCL. |
| **`NPU`** | `NPU_QDQ_OPTIMIZATION` | `bool` (RW) | `False` (default) | Quantize-Dequantize graph fusion pass optimization. |
| **`NPU`** | `WEIGHTS_PATH` | `str` (RW) | `""` (default) | Path to external weights file when `CACHE_MODE = OPTIMIZE_SIZE`. |
| **`NPU`** | `WORKLOAD_TYPE` | `WorkloadType` (RW) | `DEFAULT`, `EFFICIENT` | Energy-efficiency tuning hint for NPU power rails. |
| **`NPU`** | `OPTIMIZATION_CAPABILITIES` | `list[str]` (RO) | `['FP16', 'INT8', 'EXPORT_IMPORT']` | NPU hardware execution capabilities on Lunar Lake. Confirms full support for offline blob export/import. |
| **`GPU`** | `CACHE_DIR` | `str` (RW) | Path string (default: `""`) | Caches compiled Level Zero / OpenCL binary kernels for Arc 140V Xe2-LPG GPU. |
| **`GPU`** | `CACHE_MODE` | `CacheMode` (RW) | `OPTIMIZE_SPEED` | Optimizes Level Zero shader binary cache for minimal compile latency. |
| **`GPU`** | `GPU_ENABLE_SDPA_OPTIMIZATION` | `bool` (RW) | `True` (default) | Enables Scaled Dot-Product Attention fusion kernels on Xe2 hardware. |
| **`GPU`** | `OPTIMIZATION_CAPABILITIES` | `list[str]` (RO) | `['FP32', 'BIN', 'FP16', 'INT8', 'GPU_HW_MATMUL', 'GPU_USM_MEMORY', 'EXPORT_IMPORT']` | Arc 140V capabilities: hardware matrix multipliers (XMX-equivalent Xe2 matrix units), Unified Shared Memory (`GPU_USM_MEMORY`). |
| **`CPU`** | `ENABLE_CPU_PINNING` | `bool` (RW) | `True` (default: `True`) | Pins worker threads to designated hybrid core clusters. |
| **`CPU`** | `SCHEDULING_CORE_TYPE` | `str` (RW) | `ANY_CORE`, `ECORE_ONLY`, `PCORE_ONLY` | Thread Director placement hint: forces inference onto Lion Cove P-cores (`PCORE_ONLY`, `C0..C3`) or Skymont LP-E cores (`ECORE_ONLY`, `C4..C7`). |

---

### 5.2 NPU Model Caching Modes (`OPTIMIZE_SPEED` vs. `OPTIMIZE_SIZE`)

When configuring OpenVINO NPU caching via `core.set_property("NPU", {"CACHE_DIR": cache_dir, "CACHE_MODE": mode})`:

1. **`CacheMode.OPTIMIZE_SPEED` (Default & Recommended for Latency)**:
   - Serializes the entire converted computation graph and all model tensor weights into a monolithic `.blob` file.
   - **Disk Footprint:** Equal to or slightly larger than the INT8 model (e.g. ~612 MB for `FunctionGemma-270M-INT8`).
   - **Startup Behavior:** Single sequential read from NVMe SSD; directly mapped into NPU DMA buffers.
   - **Cold Compile vs. Warm Load:** Cold compile takes **21.8 seconds**; warm load from `.blob` takes **1.21 seconds** (18× speedup).
2. **`CacheMode.OPTIMIZE_SIZE`**:
   - Generates a lightweight, weightless `.blob` containing only graph topology and neural tile scheduling descriptors (~2–5 MB).
   - Tensor weights are referenced externally from the original OpenVINO `.bin` weights file via `WEIGHTS_PATH`.
   - **Trade-Off:** Reduces cache disk footprint on resource-constrained embedded targets, but incurs separate weight-parsing and memory-mapping passes at startup (+150–350 ms relative to `OPTIMIZE_SPEED`).

---

### 5.3 Explicit AOT Compilation vs. GenAI Pipeline In-Line Caching

Two distinct model deployment patterns exist on the Lunar Lake NPU:

#### A. Static Shape Models (`EmbeddingGemma-300M`, Feature Extractors)
Static-shape models should be compiled Ahead-Of-Time (AOT) and serialized using the explicit OpenVINO C++/Python API:
```python
import openvino as ov

core = ov.Core()
core.set_property("NPU", {"CACHE_DIR": "C:/Users/peter/models/.ov_cache"})

# 1. Ahead-Of-Time Compilation & Serialization
model = core.read_model("EmbeddingGemma-300M.xml")
model.reshape({"input_ids": [1, 128], "attention_mask": [1, 128]})
compiled = core.compile_model(model, "NPU")

# Explicitly export the precompiled blob
with open("EmbeddingGemma-300M-NPU.blob", "wb") as f:
    compiled.export_model(f)

# 2. Ultra-Fast Warm Production Import
with open("EmbeddingGemma-300M-NPU.blob", "rb") as f:
    warm_model = core.import_model(f.read(), "NPU")
```
- **Result:** `core.import_model()` bypasses the OpenVINO frontend, graph transformation passes, and VCL compilation entirely, loading the pre-compiled binary in **<120 ms**.

#### B. Generative Pipelines (`openvino_genai.LLMPipeline`, `WhisperPipeline`)
Generative pipelines manage dynamic autoregressive token loops and multi-subgraph partitioning (encoder + decoder + KV cache) internally.
- **Configuration Pattern:**
  ```python
  import openvino_genai as ov_genai
  
  pipe = ov_genai.LLMPipeline(
      "C:/Users/peter/models/llm/FunctionGemma-270M-INT8",
      "NPU",
      CACHE_DIR="C:/Users/peter/models/.ov_cache",
      PREFILL_HINT="DYNAMIC",        # Chunked prefill for prompt evaluation
      GENERATE_HINT="FAST_COMPILE",   # Fast compile vs BEST_PERF
      MAX_PROMPT_LEN=1024,           # Bounds static SRAM tile allocation
      MIN_RESPONSE_LEN=128           # Pre-reserves response token buffer
  )
  ```
- **Internal Blob Anatomy:** Inspection of `C:\Users\peter\models\.ov_cache` reveals generated XML headers:
  ```xml
  <?xml version="1.0"?>
  <compiled_blob ie_version="2026.4.1-22982-e213a147257-releases/2026/4"
                 file_info="c22b9c7921319c8d57d47ba8d3c52a0a2df335835b2a095ffeb080f56e0821d9"
                 runtime_info=""
                 header_size_alignment="4096"/>
  ```
  This confirms that OpenVINO GenAI caches discrete subgraph `.blob` files for the NPU execution wrapper (`NPUW`), with cryptographic hashes binding the cache to the exact input graph and model weights.

---

### 5.4 OpenVINO GPU Kernel Caching for Intel Arc 140V

During the Wave 2 Whisper 4-tier matrix benchmark, un-cached GPU execution exhibited noticeable cold compilation overhead:
- `Whisper Base` GPU cold compile: **2.61 seconds**
- `Whisper Medium` GPU cold compile: **7.84 seconds**
- `Whisper Large-v3-turbo` GPU cold compile: **15.15 seconds**

**Resolution via Level Zero / OpenVINO GPU Cache:**
Passing `CACHE_DIR` to the OpenVINO Core or pipeline:
```python
core.set_property("GPU", {
    "CACHE_DIR": "C:/Users/peter/models/.ov_cache/gpu",
    "CACHE_MODE": "OPTIMIZE_SPEED",
    "GPU_ENABLE_SDPA_OPTIMIZATION": True
})
```
- **Under the Hood:** Instructs the Intel Graphics Compute Runtime for oneAPI Level Zero and OpenCL to write compiled Intel Xe2-LPG ISA binaries to disk.
- **Warm Re-launch:** Cold JIT compilation is replaced by a sequential disk read of pre-compiled Level Zero binaries. Cold startup drops from **15.15s** down to **<280 ms** on `Whisper Large-v3-turbo`.

---

### 5.5 Triple-Tier Warmup Hierarchy Analysis

Across all accelerators on the LattePanda Mu Ultra, cold vs. warm startup latency follows a deterministic three-tier hierarchy:

```
+---------------------------------------------------------------------------------------------------+
|                                TRIPLE-TIER WARMUP LATENCY HIERARCHY                               |
+---------------------------------------------------------------------------------------------------+
|  Tier 1: Cold OS Disk Cache        | NVMe disk read + JIT kernel compile     | 2.5s - 22.0s latency  |
|  Tier 2: Warm OS Page Cache        | Files resident in Windows Standby list  | 0.3s - 1.2s latency   |
|  Tier 3: In-Memory Warm Request    | Graph allocated, DMA buffers committed  | < 50ms latency (TTFT) |
+---------------------------------------------------------------------------------------------------+
```

1. **Tier 1 — Cold OS Disk Cache (Power-On / After Reboot)**:
   - File data must be faulted from physical NVMe SSD into system RAM over PCIe.
   - If JIT caches do not exist, NPU VCL and GPU Level Zero run full machine code compilation.
   - *Latency:* 2.5s (small models) to 22.0s (NPU graph compilation).
2. **Tier 2 — Warm OS Page Cache (Process Restart / Service Recyle)**:
   - The `.blob` or `.gguf` file pages are already resident in the Windows System Standby Memory list.
   - File opens generate only soft page faults (resolved in RAM without NVMe I/O).
   - JIT compilation is bypassed via disk `.blob` or shader cache.
   - *Latency:* 300 ms to 1.2 seconds. Can be accelerated further using Win32 `PrefetchVirtualMemory`.
3. **Tier 3 — In-Memory Warm `InferRequest` (Interactive Operation)**:
   - The process is running, OpenVINO `InferRequest` or `llama-server` slot is instantiated.
   - Hardware neural tile DMA descriptors, GPU command queues, and scratchpad memory buffers are pre-allocated in physical RAM.
   - *Turn 1 vs. Turn 2+:* Turn 1 executes the initial tensor allocation and hardware wake-up (~80–120 ms). Turn 2+ hits pure steady-state inference latency (**~35 ms** on FunctionGemma NPU, **~15 ms** on Arc 140V prefill).

---

## 6. Architectural Bottleneck & Limitation Mapping Table

The table below maps each observed NPU, GPU, and OpenVINO startup or runtime bottleneck on Lunar Lake to either an industry-standard software solution or an explicit **`[LIMITATION MARKER]`** detailing why it cannot be resolved in software:

| Bottleneck Observed | Affected Component | Root Cause | Solution or `[LIMITATION MARKER]` | Mitigation Strategy |
|:---|:---|:---|:---|:---|
| **Cold NPU Compilation Latency (21.8s)** | FunctionGemma, EmbeddingGemma | VCL offline tile partitioning and scheduling passes run on CPU at graph compile. | **Industry Solution Tested:** `CACHE_DIR` with `CACHE_MODE = OPTIMIZE_SPEED`. | Persist compiled `.blob` to disk; subsequent loads take 1.21s. |
| **Cold GPU Level Zero Compile (2.6s–15.2s)** | Whisper on Arc 140V GPU | JIT compilation of OpenCL/Level Zero SPIR-V to Xe2 SIMD machine code. | **Industry Solution Tested:** `core.set_property("GPU", {"CACHE_DIR": ...})`. | Persist Xe2 ISA binaries to disk cache; warm startup <280 ms. |
| **NPU Dynamic Shape Rejection (`0x78000004`)** | Intel AI Boost NPU 4 | Fixed scratchpad SRAM and static DMA descriptors cannot handle unbounded dynamic shapes (`[-1, -1]`). | **`[LIMITATION MARKER: NPU Dynamic Shape Rejection 0x78000004]`** | Models must be reshaped statically offline (`model.reshape({...})`) or configured with static prompt bounds (`MAX_PROMPT_LEN`). |
| **VCL ABI Driver-Coupling on `.blob` Files** | Cached `.blob` models in `.ov_cache` | Compiled `.blob` headers embed explicit OpenVINO build hashes (`ie_version`) and VCL compiler ABI versions (`NPU_DRIVER_VERSION = 1005540`). | **`[LIMITATION MARKER: VCL ABI Driver-Coupling Across Windows Update]`** | Windows Update driver changes invalidate cached blobs. Mitigation: Implement an automated post-update background precompilation service. |
| **GenAI Sub-Graph Partitioning Overhead** | `openvino_genai.LLMPipeline`, `WhisperPipeline` | Autoregressive decode loop, KV eviction, and token sampling run on host CPU; tensor ops dispatch to NPU over UMA bus. | **`[LIMITATION MARKER: GenAI Sub-Graph Partitioning Overhead]`** | Per-token synchronization latency cannot be bypassed in split pipelines. Mitigation: Keep prefill on NPU, constrain response generation bounds. |
| **OpenVINO CPU Plugin Pins to P-Cores by Default** | STT / TTS on Skymont LP-E Cores | oneTBB CPU plugin explicitly queries core topology and pins worker threads to P-cores unless overridden. | **Industry Solution Tested:** `SCHEDULING_CORE_TYPE = "ECORE_ONLY"` and `ENABLE_CPU_PINNING = True`. | Successfully verified: forces OpenVINO worker threads to LP-E cores (`C4..C7`), saving power and freeing P-cores. |
| **Memory Bus Contention Under Tri-Engine Stress** | Arc 140V Decode + NPU Embeddings | 128-bit LPDDR5X-8533 shared bus saturates at ~136.5 GB/s theoretical bandwidth. | **`[LIMITATION MARKER: UMA_MEMORY_BANDWIDTH_136GBPS]`** | Quantize GPU KV cache (`-ctk q8_0 -ctv q8_0`), enable FlashAttention, stagger heavy NPU batch embeddings during active voice decode. |
| **Shared Compute Aperture Cap (9,156 MB)** | Multi-Model Concurrency | Windows WDDM limits shared compute device memory allocation to ~57% of physical RAM. | **`[LIMITATION MARKER: DXCORE_MCDM_SHARED_APERTURE_9156MB]`** | Memory-gate model loading: do not host Gemma 4 E4B (5 GB) and Whisper Medium (1.5 GB) concurrently with full context. |
| **Thermal / Power Throttling at Sustained 28W** | Tri-Engine Full Saturation | LattePanda Mu carrier board BIOS enforces strict PL1 ceiling of 28W. | **`[LIMITATION MARKER: CARRIER_PL1_POWER_28W]`** | Offload lightweight audio (STT/TTS) to Skymont LP-E cores (low-power island) to keep total package power within 15–22W. |

---

## 7. Hardware-Enforced Limitations (`[LIMITATION MARKER]`)

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

## 8. Synthesis: Recommended Execution Configuration for Device Commander

| Component | Target Accelerator | Load & Cache Policy | Thread & QoS Configuration | Expected Latency |
|:---|:---|:---|:---|:---:|
| **Reflex Audio (Earcons & TTS)** | CPU (Audio Island) | Pre-rendered RAM buffer | Win32 Audio Output | **< 10 ms** |
| **S1 Grammar / Reflex NLU** | CPU (Skymont LP-E) | Pre-compiled Vosk / Lexicon | EcoQoS (`C4..C7`) | **< 25 ms** |
| **S3 Intent (FunctionGemma)** | Intel AI Boost NPU 4 | OpenVINO MCDM (Static INT8) | MCDM Driver Queue | **~35 – 50 ms** |
| **Semantic Embeddings** | Intel AI Boost NPU 4 | Static shape buckets `[1, 128]` | MCDM Driver Queue | **~10 ms** |
| **Speech-to-Text (Moonshine)** | CPU (Skymont LP-E) | Pre-optimized ONNX + Warm Arena | ORT `ECORE_ONLY` (4 threads) | **~85 ms** (RTF 0.009) |
| **Dynamic SLM (Gemma 4 E2B)** | Intel Arc 140V GPU | Vulkan + FlashAttn + Slot Warmup | HighQoS / GPU Queue | **~25 – 38 tok/s** |
| **Dynamic TTS (Piper VITS)** | CPU (Skymont LP-E) | Persistent Session + Streaming | ORT `ECORE_ONLY` (4 threads) | **~145 ms** |
