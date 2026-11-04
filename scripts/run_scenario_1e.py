#!/usr/bin/env python3
"""
run_scenario_1e.py - Scenario 1E: Moonshine STT Inference on LattePanda Mu Ultra.

Coordinates exclusive execution via C:\develop\mutop\scripts\bench.lock.
Collects telemetry via mutop-ai -s 200 --mark-file ... --proc python --threads --summary.
Evaluates:
  - Moonshine tiny on Lion Cove P-cores (0x0F) vs. Skymont LP-E cores (0xF0)
  - Moonshine base on Lion Cove P-cores (0x0F) vs. Skymont LP-E cores (0xF0)
"""

import ctypes
import json
import os
import re
import subprocess
import sys
import time
from typing import Any, Dict
import numpy as np
import onnxruntime as ort


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MARK_FILE = r"C:\develop\mutop\scripts\mutop_markers.txt"
MUTOP_EXE = r"C:\develop\mutop\mutop-ai.exe"
LOG_DIR = r"C:\develop\mutop\logs"
MODELS_DIR = r"C:\Users\peter\models\stt\moonshine"


def set_cpu_affinity(mask: int) -> None:
    if os.name == "nt":
        try:
            kernel32 = ctypes.windll.kernel32
            handle = kernel32.GetCurrentProcess()
            kernel32.SetProcessAffinityMask(handle, ctypes.c_size_t(mask))
        except Exception as e:
            print(f"Warning: affinity mask 0x{mask:X} failed: {e}", file=sys.stderr)


def reset_cpu_affinity() -> None:
    set_cpu_affinity(0xFF)


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

    m_pwr = re.search(r"Board Power \(W\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_pwr:
        stats["power_median_w"] = float(m_pwr.group(2))
        stats["power_max_w"] = float(m_pwr.group(4))

    m_fan = re.search(r"Fan Speed \(RPM\)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)", content)
    if m_fan:
        stats["fan_median_rpm"] = int(m_fan.group(2))
        stats["fan_max_rpm"] = int(m_fan.group(4))

    m_cpu = re.search(r"CPU Total Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_cpu:
        stats["cpu_total_median_pct"] = float(m_cpu.group(2))

    m_big = re.search(r"Big Cores Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_big:
        stats["cpu_p_core_median_pct"] = float(m_big.group(2))

    m_lit = re.search(r"Little Cores Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_lit:
        stats["cpu_lpe_core_median_pct"] = float(m_lit.group(2))

    m_proc = re.search(r"Tracked Proc CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc:
        stats["proc_cpu_median_pct"] = float(m_proc.group(2))

    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))

    return stats


def run_cell_1e(tier: str, core_key: str, affinity_mask: int, core_desc: str) -> Dict[str, Any]:
    cell_id = f"1E_{tier}_{core_key}"
    model_path = os.path.join(MODELS_DIR, tier)
    enc_path = os.path.join(model_path, "onnx", "encoder_model.onnx")
    log_file = os.path.join(LOG_DIR, f"scenario_1e_{tier}_{core_key}.log")

    print(f"\n================================================================================")
    print(f">>> Scenario 1E: Moonshine {tier.upper()} on {core_desc}")
    print(f"================================================================================")

    if not os.path.exists(enc_path):
        print(f"Error: Moonshine ONNX model not found at {enc_path}")
        return {}

    acquire_lock(cell_id)

    with open(MARK_FILE, "w", encoding="utf-8") as f:
        f.write("")

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
    cell_results: Dict[str, Any] = {
        "tier": tier,
        "core_target": core_desc,
        "affinity_mask": f"0x{affinity_mask:02X}"
    }

    try:
        sess_opts = ort.SessionOptions()
        sess_opts.intra_op_num_threads = 4
        sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

        session = ort.InferenceSession(enc_path, sess_opts, providers=["CPUExecutionProvider"])
        input_name = session.get_inputs()[0].name

        # 10s audio
        audio_dur = 10.0
        num_samples = int(audio_dur * 16000)
        t_arr = np.linspace(0, audio_dur, num_samples, dtype=np.float32)
        test_audio = (0.5 * np.sin(2 * np.pi * 440 * t_arr)).reshape(1, -1)

        # Warmup
        reset_cpu_affinity()
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_init")
        _ = session.run(None, {input_name: test_audio})

        # Set Affinity
        set_cpu_affinity(affinity_mask)
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_infer")

        latencies = []
        for it in range(5):
            t0 = time.perf_counter()
            _ = session.run(None, {input_name: test_audio})
            t1 = time.perf_counter()
            dur = t1 - t0
            latencies.append(dur)
            print(f"    Iteration {it+1}: {dur*1000:.1f} ms (RTF: {dur/audio_dur:.3f})")

        avg_lat = float(np.mean(latencies))
        rtf = avg_lat / audio_dur

        cell_results["latency_ms"] = round(avg_lat * 1000.0, 1)
        cell_results["rtf"] = round(rtf, 4)
        cell_results["throughput_x"] = round(1.0 / rtf, 2) if rtf > 0 else 0.0

    finally:
        reset_cpu_affinity()

        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write("STREAM_END")

        for _ in range(25):
            if mutop_proc.poll() is not None:
                break
            time.sleep(0.2)

        if mutop_proc.poll() is None:
            mutop_proc.terminate()
            time.sleep(0.5)
            if mutop_proc.poll() is None:
                mutop_proc.kill()

        release_lock()

    summary_metrics = parse_summary_report(log_file)
    cell_results["telemetry"] = summary_metrics
    cell_results["log_file"] = log_file

    print(f"    Telemetry: Power: {summary_metrics.get('power_median_w', 'N/A')} W | CPU: {summary_metrics.get('cpu_total_median_pct', 'N/A')}% (P: {summary_metrics.get('cpu_p_core_median_pct', 'N/A')}%, LP-E: {summary_metrics.get('cpu_lpe_core_median_pct', 'N/A')}%)")
    return cell_results


def main():
    os.makedirs(LOG_DIR, exist_ok=True)
    all_results = {}

    targets = [
        ("cpu_p", 0x0F, "Lion Cove P-cores (C0..C3)"),
        ("cpu_lpe", 0xF0, "Skymont LP-E cores (C4..C7)"),
    ]

    for tier in ["tiny", "base"]:
        for c_key, mask, desc in targets:
            key = f"Moonshine_{tier}_{c_key}"
            res = run_cell_1e(tier, c_key, mask, desc)
            all_results[key] = res

    summary_path = os.path.join(LOG_DIR, "scenario_1e_summary.json")
    with open(summary_path, "w", encoding="utf-8") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nScenario 1E completed. Summary written to {summary_path}")


if __name__ == "__main__":
    main()
