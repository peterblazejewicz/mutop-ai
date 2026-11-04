#!/usr/bin/env python3
"""
run_scenario_1d.py - Whisper 4-Tier Tri-Engine Benchmark Runner for mutop-ai.

Executes a 4x4 benchmark matrix:
- Tiers: base, small, medium, large-v3-turbo
- Execution Targets:
  1. npu     (Intel AI Boost NPU 4)
  2. gpu     (Intel Arc 140V GPU)
  3. cpu_p   (Lion Cove P-cores 0-3, affinity 0x0F)
  4. cpu_lpe (Skymont LP-E cores 4-7, affinity 0xF0)

Coordinates exclusive hardware execution using C:\develop\mutop\scripts\bench.lock.
Collects telemetry via mutop-ai --stream 200 --mark-file ... --summary.
"""

import ctypes
import json
import os
import re
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional
import soundfile as sf
import openvino_genai as ov_genai


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MARK_FILE = r"C:\develop\mutop\scripts\mutop_markers.txt"
MUTOP_EXE = r"C:\develop\mutop\mutop-ai.exe"
LOG_DIR = r"C:\develop\mutop\logs"
MODELS_ROOT = r"C:\Users\peter\models\stt\whisper"
AUDIO_PATH = r"C:\develop\mutop\scripts\test_speech_30s.wav"
AUDIO_DURATION_SEC = 30.0

TIERS = ["base", "small", "medium", "large-v3-turbo"]
TARGETS = [
    ("npu", "NPU", 0xFF, "Intel AI Boost NPU 4"),
    ("gpu", "GPU", 0xFF, "Intel Arc 140V GPU"),
    ("cpu_p", "CPU", 0x0F, "Lion Cove P-cores (C0..C3)"),
    ("cpu_lpe", "CPU", 0xF0, "Skymont LP-E cores (C4..C7)"),
]


def set_cpu_affinity(mask: int) -> None:
    if os.name == "nt":
        try:
            kernel32 = ctypes.windll.kernel32
            handle = kernel32.GetCurrentProcess()
            kernel32.SetProcessAffinityMask(handle, ctypes.c_size_t(mask))
        except Exception as e:
            print(f"Warning: affinity mask 0x{mask:X} failed: {e}", file=sys.stderr)


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

    # Board Power
    m_pwr = re.search(r"Board Power \(W\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_pwr:
        stats["power_min_w"] = float(m_pwr.group(1))
        stats["power_median_w"] = float(m_pwr.group(2))
        stats["power_p90_w"] = float(m_pwr.group(3))
        stats["power_max_w"] = float(m_pwr.group(4))

    # Fan Speed
    m_fan = re.search(r"Fan Speed \(RPM\)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)", content)
    if m_fan:
        stats["fan_median_rpm"] = int(m_fan.group(2))
        stats["fan_max_rpm"] = int(m_fan.group(4))

    # CPU Total Load
    m_cpu = re.search(r"CPU Total Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_cpu:
        stats["cpu_total_median_pct"] = float(m_cpu.group(2))

    # Big Cores Load
    m_big = re.search(r"Big Cores Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_big:
        stats["cpu_p_core_median_pct"] = float(m_big.group(2))

    # Little Cores Load
    m_lit = re.search(r"Little Cores Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_lit:
        stats["cpu_lpe_core_median_pct"] = float(m_lit.group(2))

    # Tracked Proc CPU
    m_proc_cpu = re.search(r"Tracked Proc CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_cpu:
        stats["proc_cpu_median_pct"] = float(m_proc_cpu.group(2))

    # Proc Big Core CPU
    m_proc_big = re.search(r"Proc P/Big Core CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_big:
        stats["proc_p_core_median_pct"] = float(m_proc_big.group(2))

    # Proc Little Core CPU
    m_proc_lit = re.search(r"Proc LP-E/Little CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_lit:
        stats["proc_lpe_core_median_pct"] = float(m_proc_lit.group(2))

    # Proc Working Set
    m_ws = re.search(r"Proc Working Set \(MB\)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)", content)
    if m_ws:
        stats["proc_working_set_median_mb"] = int(m_ws.group(2))

    # Active NPU/GPU share
    m_act = re.search(r"Active Intel AI Boost NPU & Arc 140V GPU Share:\s+([\d\.]+)%", content)
    if m_act:
        stats["npu_gpu_active_share_pct"] = float(m_act.group(1))

    # Thermals
    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))
        stats["cpu_temp_max_c"] = float(m_temp.group(4))

    return stats


def run_benchmark_cell(tier: str, target_key: str, device: str, affinity_mask: int, target_desc: str, audio_data: List[float]) -> Dict[str, Any]:
    cell_id = f"1D_{tier}_{target_key}"
    print(f"\n================================================================================")
    print(f">>> Running Cell: Whisper {tier.upper():<16} on {target_desc} ({device})")
    print(f"================================================================================")

    model_dir = os.path.join(MODELS_ROOT, tier)
    log_file = os.path.join(LOG_DIR, f"scenario_1d_{tier}_{target_key}.log")
    if os.path.exists(log_file):
        try:
            os.remove(log_file)
        except Exception:
            pass

    # 1. Acquire benchmark lock
    acquire_lock(cell_id)

    # 2. Reset mark file
    with open(MARK_FILE, "w", encoding="utf-8") as f:
        f.write("")

    # 3. Launch mutop-ai collector
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

    cell_result: Dict[str, Any] = {
        "tier": tier,
        "target": target_key,
        "device": device,
        "target_desc": target_desc,
        "affinity_mask": f"0x{affinity_mask:02X}",
        "audio_duration_sec": AUDIO_DURATION_SEC,
    }

    try:
        # 4. Set CPU Affinity
        set_cpu_affinity(affinity_mask)

        # 5. Model graph load / warmup
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_init")

        print(f"    Initializing WhisperPipeline({tier}, {device})...")
        t_init0 = time.perf_counter()
        pipe = ov_genai.WhisperPipeline(model_dir, device)
        t_init1 = time.perf_counter()
        init_sec = t_init1 - t_init0
        print(f"    Initialized in {init_sec:.2f}s")
        cell_result["init_sec"] = round(init_sec, 2)

        # 6. Timed Transcription
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_infer")

        print(f"    Transcribing {AUDIO_DURATION_SEC}s audio...")
        t_infer0 = time.perf_counter()
        res = pipe.generate(audio_data)
        t_infer1 = time.perf_counter()

        latency_sec = t_infer1 - t_infer0
        latency_ms = latency_sec * 1000.0
        rtf = latency_sec / AUDIO_DURATION_SEC

        print(f"    Transcription complete in {latency_sec:.3f}s ({latency_ms:.1f}ms, RTF: {rtf:.4f})")
        if res.texts:
            print(f"    Output text: {res.texts[0][:60]}...")

        cell_result["latency_ms"] = round(latency_ms, 1)
        cell_result["latency_sec"] = round(latency_sec, 3)
        cell_result["rtf"] = round(rtf, 4)
        cell_result["sample_text"] = res.texts[0] if res.texts else ""
        cell_result["status"] = "SUCCESS"

    except Exception as e:
        print(f"    ERROR executing cell: {e}")
        cell_result["status"] = "ERROR"
        cell_result["error"] = str(e)
    finally:
        # Reset affinity
        set_cpu_affinity(0xFF)

        # Signal mutop-ai termination
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write("STREAM_END")

        # Wait for mutop-ai clean exit and summary flush
        for _ in range(25):
            if mutop_proc.poll() is not None:
                break
            time.sleep(0.2)

        if mutop_proc.poll() is None:
            mutop_proc.terminate()
            time.sleep(0.5)
            if mutop_proc.poll() is None:
                mutop_proc.kill()

        # Release lock
        release_lock()

    # 7. Extract summary metrics
    summary_metrics = parse_summary_report(log_file)
    cell_result["metrics"] = summary_metrics
    cell_result["log_file"] = log_file

    print(f"    Summary Telemetry:")
    print(f"      Board Power : median {summary_metrics.get('power_median_w', 'N/A')} W (max {summary_metrics.get('power_max_w', 'N/A')} W)")
    print(f"      NPU/GPU Act : {summary_metrics.get('npu_gpu_active_share_pct', 'N/A')} %")
    print(f"      CPU Load    : total {summary_metrics.get('cpu_total_median_pct', 'N/A')} % (P: {summary_metrics.get('cpu_p_core_median_pct', 'N/A')} %, LP-E: {summary_metrics.get('cpu_lpe_core_median_pct', 'N/A')} %)")
    print(f"      Proc CPU    : {summary_metrics.get('proc_cpu_median_pct', 'N/A')} % (P: {summary_metrics.get('proc_p_core_median_pct', 'N/A')} %, LP-E: {summary_metrics.get('proc_lpe_core_median_pct', 'N/A')} %)")
    print(f"      Fan / Temp  : {summary_metrics.get('fan_median_rpm', 'N/A')} RPM | {summary_metrics.get('cpu_temp_median_c', 'N/A')} °C")

    return cell_result


def main():
    print("================================================================================")
    print("      SCENARIO 1D: WHISPER 4-TIER TRI-ENGINE MATRIX BENCHMARK RUNNER           ")
    print("================================================================================")
    os.makedirs(LOG_DIR, exist_ok=True)

    # Load audio
    print(f"Loading benchmark audio: {AUDIO_PATH}...")
    if not os.path.exists(AUDIO_PATH):
        print(f"Error: Audio file not found at {AUDIO_PATH}", file=sys.stderr)
        sys.exit(1)

    raw_audio, sr = sf.read(AUDIO_PATH)
    audio_list = raw_audio.tolist()
    print(f"Audio loaded: {len(audio_list)} samples ({len(audio_list)/sr:.1f}s at {sr} Hz)")

    results: List[Dict[str, Any]] = []

    for tier in TIERS:
        for target_key, dev, mask, desc in TARGETS:
            res = run_benchmark_cell(tier, target_key, dev, mask, desc, audio_list)
            results.append(res)
            # Brief thermal cooldown between cells
            time.sleep(2.0)

    # Save complete matrix JSON
    matrix_path = os.path.join(LOG_DIR, "scenario_1d_matrix.json")
    with open(matrix_path, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)
    print(f"\nMatrix results saved to: {matrix_path}")

    # Print summary Markdown table
    print("\n================================================================================")
    print("               WHISPER 4x4 TRI-ENGINE BENCHMARK MATRIX REPORT                   ")
    print("================================================================================")
    header = "| Tier | Target | Device | Latency (ms) | RTF | Pkg Power (W) | NPU/GPU Act (%) | Proc CPU (%) | P-Core vs LP-E (%) | Temp (°C) | Fan (RPM) |"
    sep    = "|:-----|:-------|:-------|:-------------|:----|:--------------|:----------------|:-------------|:-------------------|:----------|:----------|"
    print(header)
    print(sep)
    for r in results:
        m = r.get("metrics", {})
        tier = r["tier"]
        target = r["target"]
        dev = r["device"]
        lat = r.get("latency_ms", "ERR")
        rtf = r.get("rtf", "ERR")
        pwr = m.get("power_median_w", "-")
        act = m.get("npu_gpu_active_share_pct", "-")
        proc_cpu = m.get("proc_cpu_median_pct", "-")
        p_vs_lpe = f"{m.get('proc_p_core_median_pct', 0.0):.1f}% / {m.get('proc_lpe_core_median_pct', 0.0):.1f}%"
        temp = m.get("cpu_temp_median_c", "-")
        fan = m.get("fan_median_rpm", "-")
        print(f"| {tier:<5} | {target:<7} | {dev:<6} | {lat!s:<12} | {rtf!s:<5} | {pwr!s:<13} | {act!s:<15} | {proc_cpu!s:<12} | {p_vs_lpe:<19} | {temp!s:<9} | {fan!s:<9} |")


if __name__ == "__main__":
    main()
