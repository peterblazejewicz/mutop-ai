#!/usr/bin/env python3
"""
verify_ecore_whisper.py - Verification test for OpenVINO oneTBB thread scheduling on Skymont LP-E cores.

Tests WhisperPipeline on CPU with:
  SCHEDULING_CORE_TYPE = "ECORE_ONLY"
  ENABLE_CPU_PINNING = True
  Process Affinity = 0xF0 (Cores 4..7)

Evaluates true Skymont LP-E latency, power, and thread placement vs default oneTBB P-Core pinning.
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
AUDIO_FILE = r"C:\develop\mutop\scripts\test_speech_30s.wav"
AUDIO_DURATION_SEC = 30.0


def acquire_lock(owner_id: str, timeout_sec: float = 600.0) -> None:
    t0 = time.time()
    while time.time() - t0 < timeout_sec:
        try:
            fd = os.open(LOCK_FILE, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            with os.fdopen(fd, "w") as f:
                f.write(f"{owner_id} [pid {os.getpid()}] at {time.strftime('%Y-%m-%d %H:%M:%S')}")
            print(f">>> [LOCK ACQUIRED] {owner_id}")
            return
        except FileExistsError:
            try:
                with open(LOCK_FILE, "r") as f:
                    holder = f.read().strip()
            except Exception:
                holder = "unknown"
            print(f"Waiting for bench.lock held by: {holder}...")
            time.sleep(1.0)
    raise TimeoutError(f"Failed to acquire {LOCK_FILE} within {timeout_sec}s")


def release_lock() -> None:
    if os.path.exists(LOCK_FILE):
        try:
            os.remove(LOCK_FILE)
            print(">>> [LOCK RELEASED]")
        except Exception as e:
            print(f"Warning: Failed to release lock file: {e}", file=sys.stderr)


def set_cpu_affinity(mask: int) -> None:
    try:
        kernel32 = ctypes.windll.kernel32
        handle = kernel32.GetCurrentProcess()
        res = kernel32.SetProcessAffinityMask(handle, ctypes.c_size_t(mask))
        if res == 0:
            print(f"Warning: SetProcessAffinityMask(0x{mask:X}) failed.")
        else:
            print(f"    Affinity set to 0x{mask:X}")
    except Exception as e:
        print(f"Warning: could not set affinity: {e}")


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

    m_big = re.search(r"Big Cores Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_big:
        stats["cpu_p_core_median_pct"] = float(m_big.group(2))

    m_lit = re.search(r"Little Cores Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_lit:
        stats["cpu_lpe_core_median_pct"] = float(m_lit.group(2))

    m_proc_cpu = re.search(r"Tracked Proc CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_cpu:
        stats["proc_cpu_median_pct"] = float(m_proc_cpu.group(2))

    m_proc_big = re.search(r"Proc P/Big Core CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_big:
        stats["proc_p_core_median_pct"] = float(m_proc_big.group(2))

    m_proc_lit = re.search(r"Proc LP-E/Little CPU \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_proc_lit:
        stats["proc_lpe_core_median_pct"] = float(m_proc_lit.group(2))

    m_ws = re.search(r"Proc Working Set \(MB\)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)", content)
    if m_ws:
        stats["proc_working_set_median_mb"] = int(m_ws.group(2))

    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))
        stats["cpu_temp_max_c"] = float(m_temp.group(4))

    return stats


def run_ecore_verification_cell(tier: str, audio_data: List[float]) -> Dict[str, Any]:
    cell_id = f"VERIFY_ECORE_{tier}"
    print(f"\n================================================================================")
    print(f">>> Running ECORE_ONLY Verification Cell: Whisper {tier.upper()} on Skymont LP-E")
    print(f"================================================================================")

    model_dir = os.path.join(MODELS_ROOT, tier)
    log_file = os.path.join(LOG_DIR, f"scenario_1d_{tier}_cpu_ecore_verified.log")
    if os.path.exists(log_file):
        try:
            os.remove(log_file)
        except Exception:
            pass

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

    cell_result: Dict[str, Any] = {
        "tier": tier,
        "target": "cpu_lpe_ecore_only",
        "scheduling_core_type": "ECORE_ONLY",
        "enable_cpu_pinning": True,
        "affinity_mask": "0xF0",
    }

    try:
        # Affinity mask to Skymont LP-E (0xF0)
        set_cpu_affinity(0xF0)

        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_init")

        print(f"    Initializing WhisperPipeline({tier}, CPU, SCHEDULING_CORE_TYPE=ECORE_ONLY)...")
        t0 = time.perf_counter()
        pipe = ov_genai.WhisperPipeline(
            model_dir,
            "CPU",
            SCHEDULING_CORE_TYPE="ECORE_ONLY",
            ENABLE_CPU_PINNING=True
        )
        t1 = time.perf_counter()
        init_sec = t1 - t0
        print(f"    Initialized in {init_sec:.2f}s")
        cell_result["init_sec"] = round(init_sec, 2)

        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"{cell_id}_infer")

        print(f"    Transcribing {AUDIO_DURATION_SEC}s audio on verified LP-E cores...")
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
        set_cpu_affinity(0xFF)

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

    metrics = parse_summary_report(log_file)
    cell_result["metrics"] = metrics
    cell_result["log_file"] = log_file

    print(f"    Summary Telemetry:")
    print(f"      Board Power : median {metrics.get('power_median_w', 'N/A')} W (max {metrics.get('power_max_w', 'N/A')} W)")
    print(f"      CPU Load    : total {metrics.get('cpu_total_median_pct', 'N/A')} % (P: {metrics.get('cpu_p_core_median_pct', 'N/A')} %, LP-E: {metrics.get('cpu_lpe_core_median_pct', 'N/A')} %)")
    print(f"      Proc CPU    : {metrics.get('proc_cpu_median_pct', 'N/A')} % (P: {metrics.get('proc_p_core_median_pct', 'N/A')} %, LP-E: {metrics.get('proc_lpe_core_median_pct', 'N/A')} %)")
    print(f"      Fan / Temp  : {metrics.get('fan_median_rpm', 'N/A')} RPM | {metrics.get('cpu_temp_median_c', 'N/A')} °C")

    return cell_result


def main():
    print("================================================================================")
    print("      OPENVINO ECORE_ONLY VERIFICATION FOR SKYMONT LP-E CORES                  ")
    print("================================================================================")
    print(f"Loading benchmark audio: {AUDIO_FILE}...")
    audio_data, sample_rate = sf.read(AUDIO_FILE, dtype="float32")
    if sample_rate != 16000:
        raise ValueError(f"Expected 16000 Hz, got {sample_rate} Hz")
    if audio_data.ndim > 1:
        audio_data = audio_data[:, 0]
    raw_audio_list = audio_data.tolist()

    results = []
    for tier in ["base", "small"]:
        res = run_ecore_verification_cell(tier, raw_audio_list)
        results.append(res)

    out_path = os.path.join(LOG_DIR, "verify_ecore_whisper.json")
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)

    print("\n================================================================================")
    print("                   ECORE_ONLY VERIFICATION COMPARISON REPORT                     ")
    print("================================================================================")
    print(f"{'Tier':<8} | {'Mode':<18} | {'Latency (ms)':<14} | {'RTF':<8} | {'Power (W)':<10} | {'Proc P-Core %':<14} | {'Proc LP-E %':<14}")
    print("-" * 92)
    # Reference numbers from default oneTBB P-Core pinning
    defaults = {
        "base": {"latency": 824.1, "rtf": 0.0275, "pwr": 6.01, "p_pct": 12.3, "lpe_pct": 0.0},
        "small": {"latency": 2740.7, "rtf": 0.0914, "pwr": 8.95, "p_pct": 39.1, "lpe_pct": 0.1},
    }
    for r in results:
        t = r["tier"]
        d = defaults[t]
        m = r.get("metrics", {})
        print(f"{t:<8} | {'Default (PCORE)':<18} | {d['latency']:<14.1f} | {d['rtf']:<8.4f} | {d['pwr']:<10.2f} | {d['p_pct']:<14.1f} | {d['lpe_pct']:<14.1f}")
        print(f"{t:<8} | {'ECORE_ONLY (LP-E)':<18} | {r.get('latency_ms', 0.0):<14.1f} | {r.get('rtf', 0.0):<8.4f} | {m.get('power_median_w', 0.0):<10.2f} | {m.get('proc_p_core_median_pct', 0.0):<14.1f} | {m.get('proc_lpe_core_median_pct', 0.0):<14.1f}")
        print("-" * 92)

    print(f"\nVerification results saved to: {out_path}")


if __name__ == "__main__":
    main()
