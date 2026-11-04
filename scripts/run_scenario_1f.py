#!/usr/bin/env python3
"""
run_scenario_1f.py - Scenario 1F: Piper Neural TTS Synthesis on LattePanda Mu Ultra.

Coordinates exclusive execution via C:\develop\mutop\scripts\bench.lock.
Collects telemetry via mutop-ai -s 200 --mark-file ... --proc python --threads --summary.
Evaluates:
  - Piper en_GB-alan-medium across Lion Cove P-cores (0x0F) vs. Skymont LP-E cores (0xF0)
  - Synthesizes 20 spoken device-commander confirmations.
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
MODELS_DIR = r"C:\Users\peter\models\tts\piper"

CONFIRMATION_PHRASES = [
    "Noise cancelling enabled.",
    "Voice feedback active.",
    "HearThrough mode turned on.",
    "Sidetone volume set to level four of five.",
    "Microphone muted.",
    "Microphone unmuted.",
    "Battery level is eighty percent.",
    "Equaliser profile set to speech clarity.",
    "Sleep timer set to thirty minutes.",
    "Busylight turned off.",
    "Hearing protection standard G616 enabled.",
    "Call audio warmer profile applied.",
    "Connected to phone and computer.",
    "Device Commander ready.",
    "Walking outside scene activated.",
    "Wind noise reduction is on.",
    "Bluetooth pairing mode started.",
    "Volume seventy percent.",
    "Headset settings synced with cloud policy.",
    "System healthy. CPU package temperature is 46 degrees."
]


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

    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))

    return stats


def text_to_phonemes(text: str, phoneme_id_map: dict) -> list:
    res = [0]
    for char in text.lower():
        val = phoneme_id_map.get(char, [1])[0]
        res.append(val)
        res.append(0)
    return res


def run_cell_1f(core_key: str, affinity_mask: int, core_desc: str) -> Dict[str, Any]:
    cell_id = f"1F_piper_{core_key}"
    onnx_path = os.path.join(MODELS_DIR, "en_GB-alan-medium.onnx")
    json_path = os.path.join(MODELS_DIR, "en_GB-alan-medium.onnx.json")
    log_file = os.path.join(LOG_DIR, f"scenario_1f_{core_key}.log")

    if not os.path.exists(onnx_path):
        onnx_path = os.path.join(MODELS_DIR, "en_GB-alba-medium.onnx")
        json_path = os.path.join(MODELS_DIR, "en_GB-alba-medium.onnx.json")

    print(f"\n================================================================================")
    print(f">>> Scenario 1F: Piper Neural TTS on {core_desc}")
    print(f"================================================================================")

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
        "core_target": core_desc,
        "affinity_mask": f"0x{affinity_mask:02X}"
    }

    try:
        with open(json_path, "r", encoding="utf-8") as f:
            cfg = json.load(f)
        phoneme_map = cfg.get("phoneme_id_map", {})
        sample_rate = cfg.get("audio", {}).get("sample_rate", 22050)

        sess_opts = ort.SessionOptions()
        sess_opts.intra_op_num_threads = 4
        sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

        session = ort.InferenceSession(onnx_path, sess_opts, providers=["CPUExecutionProvider"])
        scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)

        # Warmup
        reset_cpu_affinity()
        dummy_in = np.array([[0, 10, 0, 20, 0]], dtype=np.int64)
        _ = session.run(None, {"input": dummy_in, "input_lengths": np.array([5], dtype=np.int64), "scales": scales})

        # Set affinity
        set_cpu_affinity(affinity_mask)
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_infer")

        tot_audio = 0.0
        tot_synth = 0.0

        for idx, phrase in enumerate(CONFIRMATION_PHRASES, 1):
            p_ids = text_to_phonemes(phrase, phoneme_map)
            inp = np.array([p_ids], dtype=np.int64)
            lens = np.array([len(p_ids)], dtype=np.int64)

            t0 = time.perf_counter()
            audio = session.run(None, {"input": inp, "input_lengths": lens, "scales": scales})[0]
            t1 = time.perf_counter()

            synth_time = t1 - t0
            audio_time = audio.shape[-1] / float(sample_rate)

            tot_audio += audio_time
            tot_synth += synth_time
            rtf = synth_time / audio_time if audio_time > 0 else 0.0
            print(f"    [{idx:02d}/20] Audio: {audio_time:.2f}s | Synth: {synth_time*1000:.1f}ms | RTF: {rtf:.3f}")

        overall_rtf = tot_synth / tot_audio if tot_audio > 0 else 0.0
        cell_results["total_audio_sec"] = round(tot_audio, 2)
        cell_results["total_synth_sec"] = round(tot_synth, 2)
        cell_results["overall_rtf"] = round(overall_rtf, 4)
        cell_results["throughput_x"] = round(1.0 / overall_rtf, 2) if overall_rtf > 0 else 0.0

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

    for c_key, mask, desc in targets:
        res = run_cell_1f(c_key, mask, desc)
        all_results[f"Piper_{c_key}"] = res

    summary_path = os.path.join(LOG_DIR, "scenario_1f_summary.json")
    with open(summary_path, "w", encoding="utf-8") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nScenario 1F completed. Summary written to {summary_path}")


if __name__ == "__main__":
    main()
