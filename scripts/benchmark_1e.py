#!/usr/bin/env python3
"""
benchmark_1e.py - Scenario 1E: Moonshine STT Inference on LattePanda Mu Ultra.

Evaluates Moonshine tiny and base ONNX models across:
  - Lion Cove P-cores (affinity mask 0x0F)
  - Skymont LP-E cores (affinity mask 0xF0)
Measures:
  - Inference latency per 10s audio chunk
  - Real-Time Factor (RTF = inference_time / audio_duration)
  - Processing throughput (audio seconds / wallclock second)
  - Co-monitored by mutop-ai for core-specific CPU load, power, and thermal behavior.
"""

import argparse
import ctypes
import json
import os
import sys
import time
import numpy as np

try:
    from common_markers import MarkerWriter
except ImportError:
    from scripts.common_markers import MarkerWriter


def set_cpu_affinity(mask: int) -> None:
    """Sets process CPU affinity on Windows."""
    if os.name == "nt":
        try:
            kernel32 = ctypes.windll.kernel32
            handle = kernel32.GetCurrentProcess()
            kernel32.SetProcessAffinityMask(handle, ctypes.c_size_t(mask))
        except Exception as e:
            print(f"Warning: Failed to set CPU affinity mask 0x{mask:X}: {e}", file=sys.stderr)


def reset_cpu_affinity() -> None:
    set_cpu_affinity(0xFF)


def run_scenario_1e(models_dir: str, mark_file: Optional[str] = None, log_file: Optional[str] = None):
    writer = MarkerWriter(mark_file) if mark_file else None
    results = {}

    import onnxruntime as ort

    # Configure ONNX session options
    sess_opts = ort.SessionOptions()
    sess_opts.intra_op_num_threads = 4
    sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

    models = [
        ("tiny", os.path.join(models_dir, "stt", "moonshine", "tiny")),
        ("base", os.path.join(models_dir, "stt", "moonshine", "base")),
    ]

    audio_dur_sec = 10.0
    sample_rate = 16000
    num_samples = int(audio_dur_sec * sample_rate)
    # Generate 10s synthetic speech-band audio signal (16 kHz mono float32)
    t_arr = np.linspace(0, audio_dur_sec, num_samples, dtype=np.float32)
    synthetic_audio = (0.5 * np.sin(2 * np.pi * 440 * t_arr) + 0.25 * np.sin(2 * np.pi * 880 * t_arr)).reshape(1, -1)

    core_configs = [
        ("cpu_p", 0x0F, "Lion Cove P-cores (C0..C3)"),
        ("cpu_lpe", 0xF0, "Skymont LP-E cores (C4..C7)"),
    ]

    for model_tier, model_path in models:
        enc_onnx = os.path.join(model_path, "onnx", "encoder_model.onnx")
        if not os.path.exists(enc_onnx):
            print(f"Skipping Moonshine {model_tier}: {enc_onnx} not found.")
            continue

        print(f"\n================================================================================")
        print(f"  Loading Moonshine {model_tier.upper()} ONNX session...")
        print(f"================================================================================")
        session = ort.InferenceSession(enc_onnx, sess_opts, providers=["CPUExecutionProvider"])
        input_name = session.get_inputs()[0].name

        # Warmup
        reset_cpu_affinity()
        _ = session.run(None, {input_name: synthetic_audio})

        for core_name, affinity_mask, core_desc in core_configs:
            phase_label = f"1E_moonshine_{model_tier}_{core_name}"
            if writer:
                writer.set_marker(phase_label)

            print(f"\nEvaluating Moonshine {model_tier} on {core_desc} (affinity 0x{affinity_mask:02X})...")
            set_cpu_affinity(affinity_mask)
            try:
                latencies = []
                # Run 5 test iterations
                for it in range(5):
                    t0 = time.perf_counter()
                    _ = session.run(None, {input_name: synthetic_audio})
                    t1 = time.perf_counter()
                    dur = t1 - t0
                    latencies.append(dur)
                    print(f"  Iteration {it+1}: {dur*1000:.1f} ms (RTF: {dur/audio_dur_sec:.3f})")

                avg_latency = float(np.mean(latencies))
                median_latency = float(np.median(latencies))
                p90_latency = float(np.percentile(latencies, 90))
                rtf = avg_latency / audio_dur_sec

                results[f"Moonshine_{model_tier}_{core_name}"] = {
                    "tier": model_tier,
                    "core_target": core_desc,
                    "audio_duration_sec": audio_dur_sec,
                    "avg_latency_ms": avg_latency * 1000.0,
                    "median_latency_ms": median_latency * 1000.0,
                    "p90_latency_ms": p90_latency * 1000.0,
                    "rtf": rtf,
                    "throughput_x_realtime": 1.0 / rtf if rtf > 0 else 0.0
                }
            finally:
                reset_cpu_affinity()

    if writer:
        writer.set_marker("1E_done")

    if log_file:
        os.makedirs(os.path.dirname(os.path.abspath(log_file)), exist_ok=True)
        with open(log_file, "w", encoding="utf-8") as f:
            f.write(json.dumps(results, indent=2))
        print(f"\nSaved Scenario 1E results to {log_file}")

    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--models-dir", default=r"C:\Users\peter\models")
    parser.add_argument("--mark-file", default=None)
    parser.add_argument("--log-file", default=r"C:\develop\mutop\logs\scenario_1e_moonshine_stt.log")
    args = parser.parse_args()
    run_scenario_1e(args.models_dir, args.mark_file, args.log_file)
