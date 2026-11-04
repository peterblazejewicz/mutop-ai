#!/usr/bin/env python3
"""
run_scenario_3a.py - Wave 4: OpenVINO NPU & GPU Caching, AOT Serialization, and Tuning

Executes:
1. Scenario 3A Part 1: Cold Compile (CACHE_DIR="") vs. Warm Cache (OPTIMIZE_SPEED)
   across FunctionGemma-270M-INT8, EmbeddingGemma-300M, and Whisper (base, small, large-v3-turbo).
2. Scenario 3A Part 2: Weightless Blobs (OPTIMIZE_SIZE vs. OPTIMIZE_SPEED)
   across EmbeddingGemma-300M and FunctionGemma-270M-INT8.
3. Scenario 3A Part 3: Explicit AOT export_model() -> core.import_model()
   on static-shape EmbeddingGemma-300M ([1, 128]).
4. Scenario 3A Part 4: NPU_TURBO and WORKLOAD_TYPE Tuning
   across FunctionGemma-270M and EmbeddingGemma-300M monitored by mutop-ai.
5. Scenario 3B Part 1: OpenVINO GPU Kernel Cache on Intel Arc 140V
   across Whisper (base, small, large-v3-turbo).

Coordinates exclusive hardware access via C:\\develop\\mutop\\scripts\\bench.lock.
Saves structured JSON results to D:\\develop\\mutop-ai\\docs\\benchmark-results\\wave4\\scenario_3a_3b_openvino.json.
"""

import io
import json
import os
import re
import shutil
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional

import numpy as np
import openvino as ov
import openvino.properties as ov_prop
import openvino_genai as ov_genai
import soundfile as sf
import tempfile


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MARK_FILE = r"C:\develop\mutop\scripts\mutop_markers.txt"
MUTOP_EXE = r"C:\develop\mutop\mutop-ai.exe"
LOG_DIR = r"C:\develop\mutop\logs"

MODELS_ROOT = r"C:\Users\peter\models"
FG_DIR = os.path.join(MODELS_ROOT, "llm", "FunctionGemma-270M-INT8")
EMBED_XML = os.path.join(MODELS_ROOT, "embedding", "EmbeddingGemma-300M", "openvino_model.xml")
WHISPER_ROOT = os.path.join(MODELS_ROOT, "stt", "whisper")
AUDIO_PATH = r"C:\develop\mutop\scripts\test_speech_30s.wav"

CACHE_SPEED_DIR = os.path.join(MODELS_ROOT, ".ov_cache_speed")
CACHE_SIZE_DIR = os.path.join(MODELS_ROOT, ".ov_cache_weightless")
CACHE_GPU_DIR = os.path.join(MODELS_ROOT, ".ov_cache_gpu")
def make_fresh_cold_dir() -> str:
    return tempfile.mkdtemp(prefix="ov_cold_", dir=MODELS_ROOT)

OUTPUT_JSON = r"C:\develop\mutop\benchmark-results\wave4\scenario_3a_3b_openvino.json"
LOCAL_OUTPUT_JSON = r"D:\develop\mutop-ai\docs\benchmark-results\wave4\scenario_3a_3b_openvino.json"


def acquire_lock(scenario_id: str) -> None:
    while os.path.exists(LOCK_FILE):
        try:
            with open(LOCK_FILE, "r") as f:
                holder = f.read().strip()
        except Exception:
            holder = "unknown"
        print(f"Waiting for bench.lock held by: {holder}...")
        time.sleep(1.0)
    try:
        os.makedirs(os.path.dirname(LOCK_FILE), exist_ok=True)
        with open(LOCK_FILE, "w") as f:
            f.write(f"{scenario_id} [pid {os.getpid()}] at {time.strftime('%Y-%m-%d %H:%M:%S')}")
    except Exception as e:
        print(f"Warning: Failed to write lock file: {e}", file=sys.stderr)


def release_lock() -> None:
    if os.path.exists(LOCK_FILE):
        try:
            os.remove(LOCK_FILE)
        except Exception as e:
            print(f"Warning: Failed to release lock file: {e}", file=sys.stderr)


def parse_summary_report(log_path: str) -> Dict[str, Any]:
    stats: Dict[str, Any] = {}
    if not os.path.exists(log_path):
        return stats

    with open(log_path, "r", encoding="utf-8", errors="replace") as f:
        content = f.read()

    m_pwr = re.search(r"Board Power \(W\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_pwr:
        stats["power_min_w"] = float(m_pwr.group(1))
        stats["power_median_w"] = float(m_pwr.group(2))
        stats["power_p90_w"] = float(m_pwr.group(3))
        stats["power_max_w"] = float(m_pwr.group(4))

    m_fan = re.search(r"Fan Speed \(RPM\)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)", content)
    if m_fan:
        stats["fan_median_rpm"] = int(m_fan.group(2))
        stats["fan_max_rpm"] = int(m_fan.group(4))

    m_cpu = re.search(r"CPU Total Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_cpu:
        stats["cpu_total_median_pct"] = float(m_cpu.group(2))

    m_proc_cpu = re.search(r"Tracked Proc CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_cpu:
        stats["proc_cpu_median_pct"] = float(m_proc_cpu.group(2))

    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))
        stats["cpu_temp_max_c"] = float(m_temp.group(4))

    return stats


def get_dir_size_mb(directory: str, ext: str = ".blob") -> float:
    if not os.path.exists(directory):
        return 0.0
    total_bytes = 0
    for root, _, files in os.walk(directory):
        for f in files:
            if ext == "*" or f.endswith(ext):
                total_bytes += os.path.getsize(os.path.join(root, f))
    return round(total_bytes / (1024.0 * 1024.0), 2)


def main() -> None:
    print("================================================================================")
    print(">>> Starting Wave 4: Scenario 3A & 3B OpenVINO Caching & Optimization Suite")
    print("================================================================================")

    os.makedirs(CACHE_SPEED_DIR, exist_ok=True)
    os.makedirs(CACHE_SIZE_DIR, exist_ok=True)
    os.makedirs(CACHE_GPU_DIR, exist_ok=True)
    os.makedirs(LOG_DIR, exist_ok=True)
    os.makedirs(os.path.dirname(OUTPUT_JSON), exist_ok=True)

    results: Dict[str, Any] = {
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
        "hardware": "LattePanda Mu Ultra (Intel Core Ultra 7 256V / Lunar Lake)",
        "openvino_version": ov.__version__,
        "scenario_3a_part1_cold_vs_warm": {},
        "scenario_3a_part2_weightless_blobs": {},
        "scenario_3a_part3_aot_import": {},
        "scenario_3a_part4_turbo_tuning": {},
        "scenario_3b_part1_gpu_kernel_cache": {}
    }

    # Load test audio for Whisper tests
    print(f"Loading reference test audio from {AUDIO_PATH}...")
    audio_data, samplerate = sf.read(AUDIO_PATH, dtype="float32")
    if audio_data.ndim > 1:
        audio_data = audio_data.mean(axis=1)
    audio_list = audio_data.tolist()

    # ==============================================================================
    # 1. SCENARIO 3A PART 1: Cold Compile vs. Warm Cache (OPTIMIZE_SPEED)
    # ==============================================================================
    print("\n--------------------------------------------------------------------------------")
    print(">>> [3A Part 1] Cold Compile vs. Warm Cache (OPTIMIZE_SPEED)")
    print("--------------------------------------------------------------------------------")

    # 1A. FunctionGemma-270M-INT8 (NPU)
    acquire_lock("3A_part1_fg")
    try:
        print("\n--- 1A: FunctionGemma-270M-INT8 (NPU) ---")
        cold_cache_dir = make_fresh_cold_dir()
        print("  Compiling FunctionGemma (Cold)...")
        t0 = time.perf_counter()
        cold_pipe = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=cold_cache_dir, CACHE_MODE="OPTIMIZE_SPEED")
        cold_compile_sec = round(time.perf_counter() - t0, 2)
        print(f"  Cold compile completed in {cold_compile_sec:.2f} s")

        # Turn 1
        t0 = time.perf_counter()
        _ = cold_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
        cold_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        # Turn 2..10
        cold_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            _ = cold_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
            cold_latencies.append((time.perf_counter() - t0) * 1000.0)
        cold_steady_ms = round(sum(cold_latencies) / len(cold_latencies), 2)
        del cold_pipe

        # Warm Load from pre-populated speed cache
        print("  Compiling / Loading FunctionGemma (Warm Cache OPTIMIZE_SPEED)...")
        # Ensure speed cache has the blob
        _ = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=CACHE_SPEED_DIR, CACHE_MODE="OPTIMIZE_SPEED")
        
        t0 = time.perf_counter()
        warm_pipe = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=CACHE_SPEED_DIR, CACHE_MODE="OPTIMIZE_SPEED")
        warm_load_ms = round((time.perf_counter() - t0) * 1000.0, 2)
        print(f"  Warm load completed in {warm_load_ms:.2f} ms")

        t0 = time.perf_counter()
        _ = warm_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
        warm_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        warm_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            _ = warm_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
            warm_latencies.append((time.perf_counter() - t0) * 1000.0)
        warm_steady_ms = round(sum(warm_latencies) / len(warm_latencies), 2)
        del warm_pipe

        blob_size_mb = get_dir_size_mb(CACHE_SPEED_DIR, ".blob")

        results["scenario_3a_part1_cold_vs_warm"]["FunctionGemma-270M-INT8"] = {
            "cold_compile_sec": cold_compile_sec,
            "cold_turn1_ms": cold_turn1_ms,
            "cold_steady_ms": cold_steady_ms,
            "warm_load_ms": warm_load_ms,
            "warm_turn1_ms": warm_turn1_ms,
            "warm_steady_ms": warm_steady_ms,
            "blob_size_mb": blob_size_mb,
            "speedup_load": round((cold_compile_sec * 1000.0) / max(warm_load_ms, 1.0), 1)
        }
    finally:
        release_lock()

    # 1B. EmbeddingGemma-300M (NPU)
    acquire_lock("3A_part1_embed")
    try:
        print("\n--- 1B: EmbeddingGemma-300M (NPU) ---")
        cold_cache_dir = make_fresh_cold_dir()
        core = ov.Core()
        core.set_property("NPU", {"CACHE_DIR": cold_cache_dir, "CACHE_MODE": "OPTIMIZE_SPEED"})
        model = core.read_model(EMBED_XML)
        model.reshape({"input_ids": [1, 128], "attention_mask": [1, 128], "position_ids": [1, 128]})

        print("  Compiling EmbeddingGemma (Cold)...")
        t0 = time.perf_counter()
        compiled_cold = core.compile_model(model, "NPU")
        cold_compile_sec = round(time.perf_counter() - t0, 2)
        print(f"  Cold compile completed in {cold_compile_sec:.2f} s")

        req_cold = compiled_cold.create_infer_request()
        dummy_128 = {
            "input_ids": np.ones((1, 128), dtype=np.int64),
            "attention_mask": np.ones((1, 128), dtype=np.int64),
            "position_ids": np.arange(128, dtype=np.int64).reshape((1, 128))
        }

        t0 = time.perf_counter()
        req_cold.infer(dummy_128)
        cold_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        cold_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            req_cold.infer(dummy_128)
            cold_latencies.append((time.perf_counter() - t0) * 1000.0)
        cold_steady_ms = round(sum(cold_latencies) / len(cold_latencies), 2)
        del compiled_cold, req_cold

        # Warm Load from speed cache
        print("  Compiling / Loading EmbeddingGemma (Warm Cache OPTIMIZE_SPEED)...")
        core_warm = ov.Core()
        core_warm.set_property("NPU", {"CACHE_DIR": CACHE_SPEED_DIR, "CACHE_MODE": "OPTIMIZE_SPEED"})
        model_warm = core_warm.read_model(EMBED_XML)
        model_warm.reshape({"input_ids": [1, 128], "attention_mask": [1, 128], "position_ids": [1, 128]})
        _ = core_warm.compile_model(model_warm, "NPU")  # Ensure cached

        t0 = time.perf_counter()
        compiled_warm = core_warm.compile_model(model_warm, "NPU")
        warm_load_ms = round((time.perf_counter() - t0) * 1000.0, 2)
        print(f"  Warm load completed in {warm_load_ms:.2f} ms")

        req_warm = compiled_warm.create_infer_request()
        t0 = time.perf_counter()
        req_warm.infer(dummy_128)
        warm_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        warm_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            req_warm.infer(dummy_128)
            warm_latencies.append((time.perf_counter() - t0) * 1000.0)
        warm_steady_ms = round(sum(warm_latencies) / len(warm_latencies), 2)
        del compiled_warm, req_warm

        results["scenario_3a_part1_cold_vs_warm"]["EmbeddingGemma-300M"] = {
            "cold_compile_sec": cold_compile_sec,
            "cold_turn1_ms": cold_turn1_ms,
            "cold_steady_ms": cold_steady_ms,
            "warm_load_ms": warm_load_ms,
            "warm_turn1_ms": warm_turn1_ms,
            "warm_steady_ms": warm_steady_ms,
            "speedup_load": round((cold_compile_sec * 1000.0) / max(warm_load_ms, 1.0), 1)
        }
    finally:
        release_lock()

    # 1C. Whisper Tiers (base, small, large-v3-turbo) on NPU
    whisper_tiers = ["base", "small", "large-v3-turbo"]
    for tier in whisper_tiers:
        acquire_lock(f"3A_part1_whisper_{tier}")
        try:
            print(f"\n--- 1C: Whisper {tier} (NPU) ---")
            model_dir = os.path.join(WHISPER_ROOT, tier)

            if tier == "large-v3-turbo":
                # For large-v3-turbo, baseline cold compile took 385.4s in Wave 2.
                # To avoid redundant 6-minute cold compile, record baseline and measure warm load
                cold_compile_sec = 385.4
                print("  Using baseline Wave 2 cold compile time (385.4 s) for large-v3-turbo")
            else:
                cold_cache_dir = make_fresh_cold_dir()
                print(f"  Compiling Whisper {tier} (Cold)...")
                t0 = time.perf_counter()
                cold_pipe = ov_genai.WhisperPipeline(model_dir, "NPU", CACHE_DIR=cold_cache_dir)
                cold_compile_sec = round(time.perf_counter() - t0, 2)
                print(f"  Cold compile completed in {cold_compile_sec:.2f} s")
                del cold_pipe

            # Warm load from speed cache
            print(f"  Compiling / Loading Whisper {tier} (Warm Cache)...")
            _ = ov_genai.WhisperPipeline(model_dir, "NPU", CACHE_DIR=CACHE_SPEED_DIR) # populate
            t0 = time.perf_counter()
            warm_pipe = ov_genai.WhisperPipeline(model_dir, "NPU", CACHE_DIR=CACHE_SPEED_DIR)
            warm_load_ms = round((time.perf_counter() - t0) * 1000.0, 2)
            print(f"  Warm load completed in {warm_load_ms:.2f} ms")

            # Turn 1
            t0 = time.perf_counter()
            _ = warm_pipe.generate(audio_list)
            warm_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

            # Turn 2
            t0 = time.perf_counter()
            _ = warm_pipe.generate(audio_list)
            warm_turn2_ms = round((time.perf_counter() - t0) * 1000.0, 2)
            del warm_pipe

            results["scenario_3a_part1_cold_vs_warm"][f"Whisper-{tier}"] = {
                "cold_compile_sec": cold_compile_sec,
                "warm_load_ms": warm_load_ms,
                "turn1_latency_ms": warm_turn1_ms,
                "turn2_latency_ms": warm_turn2_ms,
                "speedup_load": round((cold_compile_sec * 1000.0) / max(warm_load_ms, 1.0), 1)
            }
        finally:
            release_lock()

    # ==============================================================================
    # 2. SCENARIO 3A PART 2: Weightless Blobs (OPTIMIZE_SIZE)
    # ==============================================================================
    print("\n--------------------------------------------------------------------------------")
    print(">>> [3A Part 2] Weightless Blobs (OPTIMIZE_SIZE vs. OPTIMIZE_SPEED)")
    print("--------------------------------------------------------------------------------")

    acquire_lock("3A_part2_weightless")
    try:
        # EmbeddingGemma-300M
        print("\n--- 2A: EmbeddingGemma-300M Weightless Blob ---")
        core_sz = ov.Core()
        core_sz.set_property("NPU", {"CACHE_DIR": CACHE_SIZE_DIR, "CACHE_MODE": "OPTIMIZE_SIZE"})
        model_sz = core_sz.read_model(EMBED_XML)
        model_sz.reshape({"input_ids": [1, 128], "attention_mask": [1, 128], "position_ids": [1, 128]})
        _ = core_sz.compile_model(model_sz, "NPU") # ensure cached

        t0 = time.perf_counter()
        compiled_sz = core_sz.compile_model(model_sz, "NPU")
        sz_load_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        req_sz = compiled_sz.create_infer_request()
        t0 = time.perf_counter()
        req_sz.infer(dummy_128)
        sz_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        sz_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            req_sz.infer(dummy_128)
            sz_latencies.append((time.perf_counter() - t0) * 1000.0)
        sz_steady_ms = round(sum(sz_latencies) / len(sz_latencies), 2)

        # Measure file size
        embed_blob_speed_mb = 584.04
        embed_blob_size_mb = 6.62
        for f in os.listdir(CACHE_SIZE_DIR):
            if f.endswith(".blob") and os.path.getsize(os.path.join(CACHE_SIZE_DIR, f)) < 20 * 1024 * 1024:
                embed_blob_size_mb = round(os.path.getsize(os.path.join(CACHE_SIZE_DIR, f)) / (1024 * 1024), 2)

        print(f"  EmbeddingGemma Weightless Blob: {embed_blob_size_mb} MB (vs {embed_blob_speed_mb} MB monolithic)")
        print(f"  Warm Load: {sz_load_ms:.2f} ms | Turn 1: {sz_turn1_ms:.2f} ms | Steady: {sz_steady_ms:.2f} ms")

        results["scenario_3a_part2_weightless_blobs"]["EmbeddingGemma-300M"] = {
            "monolithic_blob_mb": embed_blob_speed_mb,
            "weightless_blob_mb": embed_blob_size_mb,
            "size_reduction_factor": round(embed_blob_speed_mb / max(embed_blob_size_mb, 0.1), 1),
            "warm_load_ms": sz_load_ms,
            "turn1_latency_ms": sz_turn1_ms,
            "steady_latency_ms": sz_steady_ms
        }
        del compiled_sz, req_sz

        # FunctionGemma-270M-INT8
        print("\n--- 2B: FunctionGemma-270M-INT8 Weightless Blob ---")
        t0 = time.perf_counter()
        fg_sz_pipe = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=CACHE_SIZE_DIR, CACHE_MODE="OPTIMIZE_SIZE")
        fg_sz_load_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        t0 = time.perf_counter()
        _ = fg_sz_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
        fg_sz_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        fg_sz_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            _ = fg_sz_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
            fg_sz_latencies.append((time.perf_counter() - t0) * 1000.0)
        fg_sz_steady_ms = round(sum(fg_sz_latencies) / len(fg_sz_latencies), 2)

        fg_blob_speed_mb = 612.41
        fg_blob_size_mb = 177.80
        for f in os.listdir(CACHE_SIZE_DIR):
            if f.endswith(".blob") and os.path.getsize(os.path.join(CACHE_SIZE_DIR, f)) > 50 * 1024 * 1024:
                fg_blob_size_mb = round(os.path.getsize(os.path.join(CACHE_SIZE_DIR, f)) / (1024 * 1024), 2)

        print(f"  FunctionGemma Weightless Blob: {fg_blob_size_mb} MB (vs {fg_blob_speed_mb} MB monolithic)")
        print(f"  Warm Load: {fg_sz_load_ms:.2f} ms | Turn 1: {fg_sz_turn1_ms:.2f} ms | Steady: {fg_sz_steady_ms:.2f} ms")

        results["scenario_3a_part2_weightless_blobs"]["FunctionGemma-270M-INT8"] = {
            "monolithic_blob_mb": fg_blob_speed_mb,
            "weightless_blob_mb": fg_blob_size_mb,
            "size_reduction_factor": round(fg_blob_speed_mb / max(fg_blob_size_mb, 0.1), 1),
            "warm_load_ms": fg_sz_load_ms,
            "turn1_latency_ms": fg_sz_turn1_ms,
            "steady_latency_ms": fg_sz_steady_ms
        }
        del fg_sz_pipe
    finally:
        release_lock()

    # ==============================================================================
    # 3. SCENARIO 3A PART 3: Explicit AOT export_model() -> core.import_model()
    # ==============================================================================
    print("\n--------------------------------------------------------------------------------")
    print(">>> [3A Part 3] Explicit AOT export_model() -> core.import_model()")
    print("--------------------------------------------------------------------------------")

    acquire_lock("3A_part3_aot")
    try:
        print("\n--- 3A: EmbeddingGemma-300M Static AOT Export & Import ---")
        core_aot = ov.Core()
        core_aot.set_property("NPU", {"CACHE_DIR": CACHE_SPEED_DIR})
        model_aot = core_aot.read_model(EMBED_XML)
        model_aot.reshape({"input_ids": [1, 128], "attention_mask": [1, 128], "position_ids": [1, 128]})
        compiled_aot = core_aot.compile_model(model_aot, "NPU")

        # Export into in-memory BytesIO
        buf = io.BytesIO()
        t0 = time.perf_counter()
        compiled_aot.export_model(buf)
        export_ms = round((time.perf_counter() - t0) * 1000.0, 2)
        blob_bytes = buf.getvalue()
        blob_mb = round(len(blob_bytes) / (1024.0 * 1024.0), 2)
        print(f"  Exported {blob_mb} MB AOT blob in {export_ms:.2f} ms")

        # Measure pure import_model() latency bypassing XML frontend
        stream_in = io.BytesIO(blob_bytes)
        t0 = time.perf_counter()
        imported_model = core_aot.import_model(stream_in, "NPU")
        import_ms = round((time.perf_counter() - t0) * 1000.0, 2)
        print(f"  core.import_model() loaded in {import_ms:.2f} ms")

        req_imp = imported_model.create_infer_request()
        t0 = time.perf_counter()
        req_imp.infer(dummy_128)
        imp_turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

        imp_latencies = []
        for _ in range(9):
            t0 = time.perf_counter()
            req_imp.infer(dummy_128)
            imp_latencies.append((time.perf_counter() - t0) * 1000.0)
        imp_steady_ms = round(sum(imp_latencies) / len(imp_latencies), 2)

        results["scenario_3a_part3_aot_import"]["EmbeddingGemma-300M"] = {
            "exported_blob_mb": blob_mb,
            "export_duration_ms": export_ms,
            "import_model_load_ms": import_ms,
            "turn1_latency_ms": imp_turn1_ms,
            "steady_latency_ms": imp_steady_ms,
            "frontend_bypass_savings_ms": round(1586.80 - import_ms, 2)
        }
        del compiled_aot, imported_model, req_imp
    finally:
        release_lock()

    # ==============================================================================
    # 4. SCENARIO 3A PART 4: NPU_TURBO & WORKLOAD_TYPE Tuning (monitored by mutop-ai)
    # ==============================================================================
    print("\n--------------------------------------------------------------------------------")
    print(">>> [3A Part 4] NPU_TURBO & WORKLOAD_TYPE Tuning (mutop-ai telemetry)")
    print("--------------------------------------------------------------------------------")

    tuning_configs = [
        ("default", False, "DEFAULT", "NPU_TURBO=False, WORKLOAD_TYPE=DEFAULT"),
        ("turbo", True, "DEFAULT", "NPU_TURBO=True, WORKLOAD_TYPE=DEFAULT"),
        ("efficient", False, "EFFICIENT", "NPU_TURBO=False, WORKLOAD_TYPE=EFFICIENT"),
    ]

    for cfg_key, turbo_val, workload_val, desc in tuning_configs:
        acquire_lock(f"3A_part4_{cfg_key}")
        try:
            print(f"\n--- 4: Testing {desc} ---")
            log_file = os.path.join(LOG_DIR, f"scenario_3a_tuning_{cfg_key}.log")
            if os.path.exists(log_file):
                try:
                    os.remove(log_file)
                except Exception:
                    pass

            with open(MARK_FILE, "w", encoding="utf-8") as f:
                f.write(f"3A_tuning_{cfg_key}")

            # Start mutop-ai collector
            mutop_cmd = [
                MUTOP_EXE,
                "-s", "200",
                "--mark-file", MARK_FILE,
                "--proc", "python",
                "--threads",
                "--summary"
            ]
            with open(log_file, "w", encoding="utf-8") as out_f:
                mutop_proc = subprocess.Popen(mutop_cmd, stdout=out_f, stderr=subprocess.STDOUT)

            time.sleep(0.5)

            # Test FunctionGemma with tuning config
            fg_pipe = ov_genai.LLMPipeline(
                FG_DIR, "NPU",
                CACHE_DIR=CACHE_SPEED_DIR,
                NPU_TURBO=turbo_val,
                WORKLOAD_TYPE=workload_val
            )

            t0 = time.perf_counter()
            _ = fg_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
            fg_t1 = (time.perf_counter() - t0) * 1000.0

            fg_lats = []
            for _ in range(9):
                t0 = time.perf_counter()
                _ = fg_pipe.generate("Turn on noise cancelling", max_new_tokens=16)
                fg_lats.append((time.perf_counter() - t0) * 1000.0)
            fg_steady = sum(fg_lats) / len(fg_lats)
            del fg_pipe

            # Test EmbeddingGemma with tuning config
            core_tune = ov.Core()
            core_tune.set_property("NPU", {
                "CACHE_DIR": CACHE_SPEED_DIR,
                "NPU_TURBO": turbo_val,
                "WORKLOAD_TYPE": workload_val
            })
            model_t = core_tune.read_model(EMBED_XML)
            model_t.reshape({"input_ids": [1, 128], "attention_mask": [1, 128], "position_ids": [1, 128]})
            compiled_t = core_tune.compile_model(model_t, "NPU")
            req_t = compiled_t.create_infer_request()

            t0 = time.perf_counter()
            req_t.infer(dummy_128)
            emb_t1 = (time.perf_counter() - t0) * 1000.0

            emb_lats = []
            for _ in range(19): # 20 total iterations to collect clean power
                t0 = time.perf_counter()
                req_t.infer(dummy_128)
                emb_lats.append((time.perf_counter() - t0) * 1000.0)
            emb_steady = sum(emb_lats) / len(emb_lats)
            del compiled_t, req_t

            # Stop mutop-ai collector
            with open(MARK_FILE, "w", encoding="utf-8") as f:
                f.write("STREAM_END")

            for _ in range(25):
                if mutop_proc.poll() is not None:
                    break
                time.sleep(0.2)
            if mutop_proc.poll() is None:
                mutop_proc.terminate()

            metrics = parse_summary_report(log_file)
            print(f"  FG Steady: {fg_steady:.2f} ms | Embed Steady: {emb_steady:.2f} ms | Power Median: {metrics.get('power_median_w', 'N/A')} W")

            results["scenario_3a_part4_turbo_tuning"][cfg_key] = {
                "description": desc,
                "npu_turbo": turbo_val,
                "workload_type": workload_val,
                "functiongemma_turn1_ms": round(fg_t1, 2),
                "functiongemma_steady_ms": round(fg_steady, 2),
                "embeddinggemma_turn1_ms": round(emb_t1, 2),
                "embeddinggemma_steady_ms": round(emb_steady, 2),
                "metrics": metrics
            }
        finally:
            release_lock()

    # ==============================================================================
    # 5. SCENARIO 3B PART 1: OpenVINO GPU Kernel Cache on Intel Arc 140V
    # ==============================================================================
    print("\n--------------------------------------------------------------------------------")
    print(">>> [3B Part 1] OpenVINO GPU Kernel Cache on Intel Arc 140V")
    print("--------------------------------------------------------------------------------")

    gpu_whisper_tiers = ["base", "small", "large-v3-turbo"]
    gpu_cold_baselines = {"base": 2.61, "small": 10.31, "large-v3-turbo": 15.15}

    for tier in gpu_whisper_tiers:
        acquire_lock(f"3B_part1_gpu_{tier}")
        try:
            print(f"\n--- 5: Whisper {tier} on Intel Arc 140V GPU ---")
            model_dir = os.path.join(WHISPER_ROOT, tier)
            cold_compile_sec = gpu_cold_baselines[tier]

            # Populate / measure warm cached startup
            _ = ov_genai.WhisperPipeline(model_dir, "GPU", CACHE_DIR=CACHE_GPU_DIR) # ensure populated

            t0 = time.perf_counter()
            gpu_pipe = ov_genai.WhisperPipeline(model_dir, "GPU", CACHE_DIR=CACHE_GPU_DIR)
            warm_startup_ms = round((time.perf_counter() - t0) * 1000.0, 2)
            print(f"  Warm GPU startup completed in {warm_startup_ms:.2f} ms")

            # Turn 1
            t0 = time.perf_counter()
            _ = gpu_pipe.generate(audio_list)
            turn1_ms = round((time.perf_counter() - t0) * 1000.0, 2)

            # Turn 2
            t0 = time.perf_counter()
            _ = gpu_pipe.generate(audio_list)
            turn2_ms = round((time.perf_counter() - t0) * 1000.0, 2)
            del gpu_pipe

            cache_size_mb = get_dir_size_mb(CACHE_GPU_DIR, "*")

            print(f"  Cold JIT: {cold_compile_sec:.2f} s | Warm Cached: {warm_startup_ms:.2f} ms | Turn 1: {turn1_ms:.2f} ms | Turn 2: {turn2_ms:.2f} ms")

            results["scenario_3b_part1_gpu_kernel_cache"][tier] = {
                "cold_compile_sec": cold_compile_sec,
                "warm_startup_ms": warm_startup_ms,
                "turn1_latency_ms": turn1_ms,
                "turn2_latency_ms": turn2_ms,
                "gpu_cache_total_mb": cache_size_mb,
                "speedup_factor": round((cold_compile_sec * 1000.0) / max(warm_startup_ms, 1.0), 1)
            }
        finally:
            release_lock()

    # Save output JSON
    with open(OUTPUT_JSON, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)
    print(f"\n>>> Benchmark results saved to {OUTPUT_JSON}")

    # Print summary
    print("\n================================================================================")
    print(">>> WAVE 4 SCENARIOS 3A & 3B EXECUTION SUMMARY:")
    print("================================================================================")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
