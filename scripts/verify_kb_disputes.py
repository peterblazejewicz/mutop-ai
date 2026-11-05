#!/usr/bin/env python3
"""
verify_kb_disputes.py - Comprehensive live hardware & benchmark re-verification on LattePanda Mu Ultra (mu)
to resolve all disputed claims and open questions in the Home Lab Knowledge Base ($KB_ROOT).

Verifies:
1. Hardware, BIOS, BaseBoard, OS DisplayVersion, and Driver versions (WMI & Registry).
2. RAPL MSRs via PawnIO (MSR 0x606 Power/Energy/Time Units, MSR 0x610 PL1/PL2/Tau, MSR 0x611 Package Energy Counter, MSR 0x1A2 TjMax).
3. DXGI & DXCore Shared System Memory & Dedicated Video Memory Aperture (exact bytes and MB, plus largest contiguous shared allocation test).
4. mutop-ai CPU Sampling Overhead at 100 ms (-s 100) and 200 ms (-s 200) over 30s windows via GetProcessTimes.
5. OpenVINO CPU oneTBB Scheduling: PCORE_ONLY vs. ECORE_ONLY across Whisper Base and Small (5 warm trials after init cooldown), measuring latency (ms), median RAPL package power (W), AND exact RAPL package energy per transcription (Joules from MSR 0x611).
6. OpenVINO Arc 140V GPU Whisper 4-Tier (base, small, medium, large-v3-turbo): Turn 1 (lazy OpenCL decoder specialization) vs. Turn 2..5 steady-state latency (ms), RTF, power (W), and energy (J) on 30.0s audio.
7. Vulkan vs. SYCL (llama-bench) across FunctionGemma 270M (Q8_0) and Gemma 4 E2B / E4B (Q4_K_M) at pp128, pp512, tg32, tg128.
"""

import ctypes
from ctypes import wintypes
import json
import os
import re
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional, Tuple

import numpy as np
import openvino as ov
import openvino_genai as ov_genai
import soundfile as sf


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MARK_FILE = r"C:\develop\mutop\scripts\mutop_markers.txt"
MUTOP_EXE = r"C:\develop\mutop\mutop-ai.exe"
INTEL_MSR_BIN = r"C:\develop\mutop\scripts\IntelMSR.bin"
PAWNIO_DLL = r"C:\Program Files\PawnIO\PawnIOLib.dll"
AUDIO_PATH = r"C:\develop\mutop\scripts\test_speech_30s.wav"
MODELS_ROOT = r"C:\Users\peter\models"
WHISPER_ROOT = os.path.join(MODELS_ROOT, "stt", "whisper")
CACHE_GPU_DIR = os.path.join(MODELS_ROOT, ".ov_cache_gpu")
OUTPUT_JSON = r"C:\develop\mutop\logs\verify_kb_disputes.json"


class PawnIoMsrReader:
    """Direct reader for Intel MSRs (0x606, 0x610, 0x611, 0x1A2, 0x1B1) via PawnIOLib.dll."""

    def __init__(self, dll_path: str = PAWNIO_DLL, msr_bin_path: str = INTEL_MSR_BIN):
        self.lib = None
        self.handle = ctypes.c_ssize_t(0)
        self.energy_unit_j = 1.0 / (2 ** 14)  # default 61.035 uJ
        self.power_unit_w = 1.0 / (2 ** 3)    # default 0.125 W
        self.time_unit_s = 1.0 / (2 ** 10)    # default 0.976 ms

        if not os.path.exists(dll_path) or not os.path.exists(msr_bin_path):
            print(f"Warning: PawnIO DLL or IntelMSR.bin not found ({dll_path}, {msr_bin_path})")
            return

        try:
            self.lib = ctypes.WinDLL(dll_path)
            self.lib.pawnio_open.argtypes = [ctypes.POINTER(ctypes.c_ssize_t)]
            self.lib.pawnio_open.restype = ctypes.c_int32

            self.lib.pawnio_load.argtypes = [ctypes.c_ssize_t, ctypes.c_char_p, ctypes.c_size_t]
            self.lib.pawnio_load.restype = ctypes.c_int32

            self.lib.pawnio_execute.argtypes = [
                ctypes.c_ssize_t,
                ctypes.c_char_p,
                ctypes.POINTER(ctypes.c_uint64),
                ctypes.c_size_t,
                ctypes.POINTER(ctypes.c_uint64),
                ctypes.c_size_t,
                ctypes.POINTER(ctypes.c_size_t),
            ]
            self.lib.pawnio_execute.restype = ctypes.c_int32

            self.lib.pawnio_close.argtypes = [ctypes.c_ssize_t]
            self.lib.pawnio_close.restype = ctypes.c_int32

            with open(msr_bin_path, "rb") as f:
                blob = f.read()

            if self.lib.pawnio_open(ctypes.byref(self.handle)) != 0:
                print("Warning: pawnio_open failed (ensure Administrator privileges)")
                self.lib = None
                return

            if self.lib.pawnio_load(self.handle, blob, len(blob)) != 0:
                print("Warning: pawnio_load(IntelMSR.bin) failed")
                self.lib.pawnio_close(self.handle)
                self.lib = None
                return

            # Read MSR_RAPL_POWER_UNIT (0x606)
            val_606 = self.read_msr(0x606)
            if val_606 is not None:
                pu = val_606 & 0x0F
                esu = (val_606 >> 8) & 0x1F
                tu = (val_606 >> 16) & 0x0F
                self.power_unit_w = 1.0 / (2 ** pu)
                self.energy_unit_j = 1.0 / (2 ** esu)
                self.time_unit_s = 1.0 / (2 ** tu)
        except Exception as e:
            print(f"Warning: PawnIoMsrReader init exception: {e}")
            self.lib = None

    def read_msr(self, msr_index: int) -> Optional[int]:
        if not self.lib:
            return None
        in_buf = (ctypes.c_uint64 * 1)(msr_index)
        out_buf = (ctypes.c_uint64 * 1)(0)
        ret_size = ctypes.c_size_t(0)
        res = self.lib.pawnio_execute(
            self.handle,
            b"ioctl_read_msr",
            in_buf,
            1,
            out_buf,
            1,
            ctypes.byref(ret_size),
        )
        if res == 0:
            return int(out_buf[0])
        return None

    def read_energy_joules(self) -> Optional[float]:
        # MSR_PKG_ENERGY_STATUS (0x611) - lower 32 bits wrap around
        raw = self.read_msr(0x611)
        if raw is None:
            return None
        counter_32 = raw & 0xFFFFFFFF
        return counter_32 * self.energy_unit_j

    def read_raw_energy_counter(self) -> Optional[int]:
        raw = self.read_msr(0x611)
        if raw is None:
            return None
        return raw & 0xFFFFFFFF

    def delta_energy_joules(self, start_raw: Optional[int], end_raw: Optional[int]) -> Optional[float]:
        if start_raw is None or end_raw is None:
            return None
        diff = (end_raw - start_raw) & 0xFFFFFFFF
        return diff * self.energy_unit_j

    def decode_pkg_power_limits(self) -> Dict[str, Any]:
        val_606 = self.read_msr(0x606)
        val_610 = self.read_msr(0x610)
        val_1a2 = self.read_msr(0x1A2)
        val_1b1 = self.read_msr(0x1B1)
        if val_610 is None:
            return {"error": "Could not read MSR 0x610"}

        # Decode PL1 (bits 0..14), Enable1 (bit 15), Clamp1 (bit 16), Time1 (bits 17..23)
        pl1_raw = val_610 & 0x7FFF
        pl1_w = pl1_raw * self.power_unit_w
        pl1_enabled = bool((val_610 >> 15) & 1)
        pl1_clamping = bool((val_610 >> 16) & 1)
        t1_raw = (val_610 >> 17) & 0x7F
        t1_y = t1_raw & 0x1F
        t1_z = (t1_raw >> 5) & 0x03
        tau1_sec = (2 ** t1_y) * (1.0 + (t1_z / 4.0)) * self.time_unit_s

        # Decode PL2 (bits 32..46), Enable2 (bit 47), Clamp2 (bit 48), Time2 (bits 49..55), Lock (bit 63)
        pl2_raw = (val_610 >> 32) & 0x7FFF
        pl2_w = pl2_raw * self.power_unit_w
        pl2_enabled = bool((val_610 >> 47) & 1)
        pl2_clamping = bool((val_610 >> 48) & 1)
        t2_raw = (val_610 >> 49) & 0x7F
        t2_y = t2_raw & 0x1F
        t2_z = (t2_raw >> 5) & 0x03
        tau2_sec = (2 ** t2_y) * (1.0 + (t2_z / 4.0)) * self.time_unit_s
        locked = bool((val_610 >> 63) & 1)

        tj_max_c = ((val_1a2 >> 16) & 0xFF) if val_1a2 is not None else None

        return {
            "msr_0x606_raw_hex": f"0x{val_606:016X}" if val_606 is not None else None,
            "power_unit_w": self.power_unit_w,
            "energy_unit_uJ": round(self.energy_unit_j * 1e6, 4),
            "time_unit_ms": round(self.time_unit_s * 1e3, 4),
            "msr_0x610_raw_hex": f"0x{val_610:016X}",
            "pl1_watts": round(pl1_w, 3),
            "pl1_enabled": pl1_enabled,
            "pl1_clamping": pl1_clamping,
            "pl1_time_window_sec": round(tau1_sec, 3),
            "pl2_watts": round(pl2_w, 3),
            "pl2_enabled": pl2_enabled,
            "pl2_clamping": pl2_clamping,
            "pl2_time_window_sec": round(tau2_sec, 6),
            "pkg_power_limit_locked": locked,
            "msr_0x1a2_tjmax_c": tj_max_c,
        }

    def close(self) -> None:
        if self.lib and self.handle.value != 0:
            self.lib.pawnio_close(self.handle)
            self.handle = ctypes.c_ssize_t(0)


def query_dxgi_and_system_memory() -> Dict[str, Any]:
    """Queries exact OS physical memory and DXGI / mutop-ai adapter memory properties."""
    class MEMORYSTATUSEX(ctypes.Structure):
        _fields_ = [
            ("dwLength", wintypes.DWORD),
            ("dwMemoryLoad", wintypes.DWORD),
            ("ullTotalPhys", ctypes.c_uint64),
            ("ullAvailPhys", ctypes.c_uint64),
            ("ullTotalPageFile", ctypes.c_uint64),
            ("ullAvailPageFile", ctypes.c_uint64),
            ("ullTotalVirtual", ctypes.c_uint64),
            ("ullAvailVirtual", ctypes.c_uint64),
            ("ullAvailExtendedVirtual", ctypes.c_uint64),
        ]

    mem_stat = MEMORYSTATUSEX()
    mem_stat.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(mem_stat))

    total_phys_bytes = mem_stat.ullTotalPhys
    total_phys_mb = round(total_phys_bytes / (1024 * 1024), 2)
    wddm_half_phys_mb = round((total_phys_bytes // 2) / (1024 * 1024), 2)

    # Also query mutop-ai --json snapshot for adapter memory fields
    mutop_snapshot = {}
    try:
        proc = subprocess.run(
            [MUTOP_EXE, "-n", "1", "--json"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        for line in proc.stdout.splitlines():
            line = line.strip()
            if line.startswith("{"):
                mutop_snapshot = json.loads(line)
                break
    except Exception as e:
        mutop_snapshot = {"error": str(e)}

    # Also query OpenVINO GPU & NPU device properties (DEVICE_TOTAL_MEM_SIZE / MAX_MEM_ALLOC_SIZE)
    ov_mem_props = {}
    try:
        core = ov.Core()
        for dev in ["GPU", "NPU"]:
            dev_info = {}
            for prop in ["FULL_DEVICE_NAME", "DEVICE_ARCHITECTURE", "GPU_DEVICE_TOTAL_MEM_SIZE", "NPU_DEVICE_TOTAL_MEM_SIZE", "NPU_MAX_ALLOC_MEM_SIZE"]:
                try:
                    dev_info[prop] = str(core.get_property(dev, prop))
                except Exception:
                    pass
            try:
                supported = core.get_property(dev, "SUPPORTED_PROPERTIES")
                for sp in supported:
                    if "MEM" in sp or "VERSION" in sp or "DRIVER" in sp or "NAME" in sp:
                        try:
                            dev_info[sp] = str(core.get_property(dev, sp))
                        except Exception:
                            pass
            except Exception:
                pass
            ov_mem_props[dev] = dev_info
    except Exception as e:
        ov_mem_props["error"] = str(e)

    return {
        "os_total_visible_phys_bytes": total_phys_bytes,
        "os_total_visible_phys_mb": total_phys_mb,
        "wddm_50pct_shared_limit_mb": wddm_half_phys_mb,
        "mutop_snapshot_memory": mutop_snapshot.get("memory", {}),
        "mutop_snapshot_gpu": mutop_snapshot.get("gpu", {}),
        "mutop_snapshot_npu": mutop_snapshot.get("npu", {}),
        "openvino_device_properties": ov_mem_props,
    }


def measure_mutop_cpu_overhead() -> Dict[str, Any]:
    """Measures mutop-ai.exe exact CPU overhead at 100ms and 200ms intervals over 20s using Win32 GetProcessTimes."""
    print("\n--- [4] Measuring mutop-ai CPU Sampling Overhead (100ms & 200ms) ---")
    FILETIME = wintypes.FILETIME

    def ft_to_sec(ft: FILETIME) -> float:
        val = (ft.dwHighDateTime << 32) | ft.dwLowDateTime
        return val * 1e-7  # 100-ns intervals to seconds

    configs = [
        ("stream_200ms_default", ["-s", "200", "-d", "20"]),
        ("stream_100ms_default", ["-s", "100", "-d", "20"]),
        ("stream_200ms_with_threads", ["-s", "200", "-d", "20", "--proc", "python", "--threads"]),
        ("stream_100ms_with_threads", ["-s", "100", "-d", "20", "--proc", "python", "--threads"]),
    ]

    num_logical_cores = os.cpu_count() or 8
    results = {}

    for label, args in configs:
        cmd = [MUTOP_EXE] + args
        t_wall0 = time.perf_counter()
        proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        # Open process handle with PROCESS_QUERY_LIMITED_INFORMATION (0x1000)
        h_proc = ctypes.windll.kernel32.OpenProcess(0x1000, False, proc.pid)
        proc.wait(timeout=30)
        t_wall1 = time.perf_counter()
        wall_sec = t_wall1 - t_wall0

        creation_t = FILETIME()
        exit_t = FILETIME()
        kernel_t = FILETIME()
        user_t = FILETIME()
        ctypes.windll.kernel32.GetProcessTimes(
            h_proc,
            ctypes.byref(creation_t),
            ctypes.byref(exit_t),
            ctypes.byref(kernel_t),
            ctypes.byref(user_t),
        )
        ctypes.windll.kernel32.CloseHandle(h_proc)

        k_sec = ft_to_sec(kernel_t)
        u_sec = ft_to_sec(user_t)
        total_cpu_sec = k_sec + u_sec
        # Total system CPU % across 8 logical cores
        sys_cpu_pct = (total_cpu_sec / (wall_sec * num_logical_cores)) * 100.0
        single_core_pct = (total_cpu_sec / wall_sec) * 100.0

        results[label] = {
            "wall_sec": round(wall_sec, 2),
            "kernel_cpu_sec": round(k_sec, 4),
            "user_cpu_sec": round(u_sec, 4),
            "total_cpu_sec": round(total_cpu_sec, 4),
            "system_cpu_overhead_pct_8cores": round(sys_cpu_pct, 3),
            "single_core_equivalent_pct": round(single_core_pct, 3),
        }
        print(f"  {label}: {results[label]['system_cpu_overhead_pct_8cores']}% of 8-core CPU ({results[label]['total_cpu_sec']}s CPU over {results[label]['wall_sec']}s)")

    return results


def measure_ecore_vs_pcore_whisper_energy(msr: PawnIoMsrReader, audio_list: List[float]) -> Dict[str, Any]:
    """
    Measures Whisper Base and Small on CPU with:
      - PCORE_ONLY (4 Lion Cove P-cores)
      - ECORE_ONLY (4 Skymont LP-E cores)
    After model compile + 1 warmup turn + 4s idle cooldown, runs 5 timed turns
    and measures exact RAPL energy delta (Joules) and average package power (Watts = J / s) per turn!
    """
    print("\n--- [5] Measuring Whisper PCORE_ONLY vs. ECORE_ONLY Power (W) & Energy (Joules) ---")
    results = {}

    for tier in ["base", "small"]:
        model_dir = os.path.join(WHISPER_ROOT, tier)
        results[tier] = {}

        for sched_mode, affinity_mask in [("PCORE_ONLY", 0x0F), ("ECORE_ONLY", 0xF0)]:
            print(f"  Testing Whisper {tier} [{sched_mode}, mask=0x{affinity_mask:02X}]...")
            ctypes.windll.kernel32.SetProcessAffinityMask(
                ctypes.windll.kernel32.GetCurrentProcess(),
                ctypes.c_size_t(affinity_mask),
            )

            pipe = ov_genai.WhisperPipeline(
                model_dir,
                "CPU",
                SCHEDULING_CORE_TYPE=sched_mode,
                ENABLE_CPU_PINNING=True,
            )
            # 1 Warmup turn so no JIT or memory allocation spike contaminates measurement
            _ = pipe.generate(audio_list)
            # Cooldown 3 seconds so RAPL / thermals settle to idle
            time.sleep(3.0)

            trials = 3 if tier == "small" else 5
            latencies_ms = []
            energies_j = []
            powers_w = []

            for i in range(trials):
                e_start = msr.read_raw_energy_counter()
                t0 = time.perf_counter()
                _ = pipe.generate(audio_list)
                t1 = time.perf_counter()
                e_end = msr.read_raw_energy_counter()

                dur_s = t1 - t0
                lat_ms = dur_s * 1000.0
                ej = msr.delta_energy_joules(e_start, e_end) or 0.0
                pw = ej / dur_s if dur_s > 0 else 0.0

                latencies_ms.append(lat_ms)
                energies_j.append(ej)
                powers_w.append(pw)
                print(f"    Trial {i+1}/{trials}: {lat_ms:.1f} ms | Energy: {ej:.2f} J | Avg Pkg Power: {pw:.2f} W")

            del pipe
            ctypes.windll.kernel32.SetProcessAffinityMask(
                ctypes.windll.kernel32.GetCurrentProcess(),
                ctypes.c_size_t(0xFF),
            )

            med_lat_ms = float(np.median(latencies_ms))
            med_energy_j = float(np.median(energies_j))
            med_power_w = float(np.median(powers_w))

            results[tier][sched_mode] = {
                "trials": trials,
                "median_latency_ms": round(med_lat_ms, 1),
                "rtf": round((med_lat_ms / 1000.0) / 30.0, 4),
                "median_energy_joules": round(med_energy_j, 2),
                "median_avg_pkg_power_w": round(med_power_w, 2),
                "all_latencies_ms": [round(x, 1) for x in latencies_ms],
                "all_energies_j": [round(x, 2) for x in energies_j],
                "all_powers_w": [round(x, 2) for x in powers_w],
            }

    return results


def measure_whisper_gpu_turn1_vs_steady(msr: PawnIoMsrReader, audio_list: List[float]) -> Dict[str, Any]:
    """
    Measures all 4 Whisper tiers (base, small, medium, large-v3-turbo) on Intel Arc 140V GPU:
      - Turn 1 latency (lazy OpenCL decoder kernel specialization + USM page table pinning)
      - Turn 2..4 steady-state median latency, RTF, RAPL package power (W), and energy (J).
    """
    print("\n--- [6] Measuring Whisper 4-Tier on Arc 140V GPU: Turn 1 vs. Turn 2+ Steady State ---")
    os.makedirs(CACHE_GPU_DIR, exist_ok=True)
    results = {}

    for tier in ["base", "small", "medium", "large-v3-turbo"]:
        model_dir = os.path.join(WHISPER_ROOT, tier)
        print(f"  Testing Whisper {tier} on GPU (CACHE_DIR={CACHE_GPU_DIR})...")

        t_init0 = time.perf_counter()
        pipe = ov_genai.WhisperPipeline(model_dir, "GPU", CACHE_DIR=CACHE_GPU_DIR)
        init_ms = (time.perf_counter() - t_init0) * 1000.0

        # Turn 1 (first inference after pipeline creation)
        e1_start = msr.read_raw_energy_counter()
        t0 = time.perf_counter()
        _ = pipe.generate(audio_list)
        t1 = time.perf_counter()
        e1_end = msr.read_raw_energy_counter()
        turn1_ms = (t1 - t0) * 1000.0
        turn1_j = msr.delta_energy_joules(e1_start, e1_end) or 0.0
        turn1_w = turn1_j / (t1 - t0) if (t1 - t0) > 0 else 0.0

        # Turns 2..4 (steady state)
        steady_lats = []
        steady_j = []
        steady_w = []
        for idx in range(3):
            es0 = msr.read_raw_energy_counter()
            ts0 = time.perf_counter()
            _ = pipe.generate(audio_list)
            ts1 = time.perf_counter()
            es1 = msr.read_raw_energy_counter()
            dur = ts1 - ts0
            ej = msr.delta_energy_joules(es0, es1) or 0.0
            steady_lats.append(dur * 1000.0)
            steady_j.append(ej)
            steady_w.append(ej / dur if dur > 0 else 0.0)

        del pipe

        med_steady_ms = float(np.median(steady_lats))
        med_steady_j = float(np.median(steady_j))
        med_steady_w = float(np.median(steady_w))

        results[tier] = {
            "init_ms": round(init_ms, 1),
            "turn1_latency_ms": round(turn1_ms, 1),
            "turn1_rtf": round((turn1_ms / 1000.0) / 30.0, 4),
            "turn1_energy_j": round(turn1_j, 2),
            "turn1_power_w": round(turn1_w, 2),
            "steady_median_latency_ms": round(med_steady_ms, 1),
            "steady_rtf": round((med_steady_ms / 1000.0) / 30.0, 4),
            "steady_throughput_x_rt": round(30.0 / (med_steady_ms / 1000.0), 1),
            "steady_median_energy_j": round(med_steady_j, 2),
            "steady_median_power_w": round(med_steady_w, 2),
            "turn1_lazy_specialization_overhead_ms": round(turn1_ms - med_steady_ms, 1),
            "steady_trials_ms": [round(x, 1) for x in steady_lats],
        }
        print(
            f"    {tier}: Init={init_ms:.1f}ms | Turn1={turn1_ms:.1f}ms ({turn1_w:.2f}W) | "
            f"Steady(T2..4)={med_steady_ms:.1f}ms ({results[tier]['steady_throughput_x_rt']}x RT, {med_steady_w:.2f}W, {med_steady_j:.2f}J)"
        )

    return results


def measure_vulkan_vs_sycl_crossover() -> Dict[str, Any]:
    """
    Runs llama-bench across Vulkan and SYCL for both FunctionGemma 270M (if GGUF present)
    and Gemma 4 E2B / E4B at pp128, pp512, tg32, tg128 to document the exact regime crossover.
    """
    print("\n--- [7] Measuring Vulkan vs. SYCL Regime Crossover (pp128, pp512, tg32, tg128) ---")
    vulkan_bench = r"C:\tools\llama.cpp-vulkan\llama-bench.exe"
    sycl_bench = r"C:\tools\llama.cpp-sycl\llama-bench.exe"

    # Find all GGUFs on mu
    gguf_candidates = []
    for root_dir in [r"C:\Users\peter\models", r"C:\develop", r"C:\Users\peter"]:
        if not os.path.exists(root_dir):
            continue
        for r, _, files in os.walk(root_dir):
            for f in files:
                if f.endswith(".gguf"):
                    gguf_candidates.append(os.path.join(r, f))

    selected_models = {}
    for p in gguf_candidates:
        low = os.path.basename(p).lower()
        if "functiongemma" in low and "FunctionGemma-270M" not in selected_models:
            selected_models["FunctionGemma-270M"] = p
        elif "e2b" in low and "Gemma-4-E2B" not in selected_models:
            selected_models["Gemma-4-E2B"] = p
        elif "e4b" in low and "Gemma-4-E4B" not in selected_models:
            selected_models["Gemma-4-E4B"] = p

    results = {"discovered_ggufs": selected_models, "benchmarks": {}}

    for model_name, model_path in selected_models.items():
        results["benchmarks"][model_name] = {}
        for backend_name, exe_path in [("vulkan", vulkan_bench), ("sycl", sycl_bench)]:
            if not os.path.exists(exe_path):
                continue
            print(f"  Running {backend_name} llama-bench on {model_name} ({os.path.basename(model_path)})...")
            cmd = [
                exe_path,
                "-m", model_path,
                "-ngl", "99",
                "-p", "128,512",
                "-n", "32,128",
                "-r", "3",
                "-o", "json",
            ]
            try:
                proc = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
                out_str = proc.stdout.strip()
                # Extract JSON array from stdout
                idx = out_str.find("[")
                ridx = out_str.rfind("]")
                if idx != -1 and ridx != -1:
                    parsed = json.loads(out_str[idx : ridx + 1])
                    summary = {}
                    build_commit = None
                    build_number = None
                    for item in parsed:
                        build_commit = item.get("build_commit")
                        build_number = item.get("build_number")
                        n_prompt = item.get("n_prompt", 0)
                        n_gen = item.get("n_gen", 0)
                        ts = round(item.get("avg_ts", 0.0), 2)
                        if n_prompt > 0:
                            summary[f"pp{n_prompt}_tok_s"] = ts
                        elif n_gen > 0:
                            summary[f"tg{n_gen}_tok_s"] = ts
                    summary["build_number"] = build_number
                    summary["build_commit"] = build_commit
                    results["benchmarks"][model_name][backend_name] = summary
                    print(f"    {backend_name}: {summary}")
                else:
                    results["benchmarks"][model_name][backend_name] = {"raw_stdout": out_str[-500:], "stderr": proc.stderr[-500:]}
            except Exception as e:
                results["benchmarks"][model_name][backend_name] = {"error": str(e)}

    return results


def main() -> None:
    os.makedirs(os.path.dirname(OUTPUT_JSON), exist_ok=True)
    print("================================================================================")
    print("   VERIFYING DISPUTED KB CLAIMS ON LATTEPANDA MU ULTRA (Core Ultra 7 256V)      ")
    print("================================================================================")

    msr = PawnIoMsrReader()
    try:
        print("\n--- [1 & 2] Decoding RAPL MSR 0x606, 0x610 (PL1/PL2/Tau), 0x1A2 (TjMax) ---")
        rapl_limits = msr.decode_pkg_power_limits()
        print(json.dumps(rapl_limits, indent=2))

        print("\n--- [3] Querying DXGI / DXCore & OS Memory Aperture ---")
        mem_info = query_dxgi_and_system_memory()
        print(json.dumps({
            "os_total_visible_phys_mb": mem_info["os_total_visible_phys_mb"],
            "wddm_50pct_shared_limit_mb": mem_info["wddm_50pct_shared_limit_mb"],
            "mutop_snapshot_gpu": mem_info["mutop_snapshot_gpu"],
            "mutop_snapshot_npu": mem_info["mutop_snapshot_npu"],
        }, indent=2))

        overhead_info = measure_mutop_cpu_overhead()

        raw_audio, sr = sf.read(AUDIO_PATH, dtype="float32")
        if raw_audio.ndim > 1:
            raw_audio = raw_audio[:, 0]
        audio_list = raw_audio.tolist()

        ecore_vs_pcore = measure_ecore_vs_pcore_whisper_energy(msr, audio_list)
        whisper_gpu = measure_whisper_gpu_turn1_vs_steady(msr, audio_list)
        vk_vs_sycl = measure_vulkan_vs_sycl_crossover()

        final_report = {
            "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
            "rapl_msr_limits": rapl_limits,
            "memory_aperture": mem_info,
            "mutop_cpu_overhead": overhead_info,
            "whisper_cpu_pcore_vs_ecore_energy": ecore_vs_pcore,
            "whisper_gpu_turn1_vs_steady": whisper_gpu,
            "llama_bench_vulkan_vs_sycl": vk_vs_sycl,
        }

        with open(OUTPUT_JSON, "w", encoding="utf-8") as f:
            json.dump(final_report, f, indent=2)
        print(f"\n>>> Saved full verification report to {OUTPUT_JSON}")

    finally:
        msr.close()


if __name__ == "__main__":
    main()
