#!/usr/bin/env python3
"""
benchmark_1f.py - Scenario 1F: Piper Neural TTS Synthesis on LattePanda Mu Ultra.

Evaluates Piper VITS ONNX model (en_GB-alan-medium) across:
  - Lion Cove P-cores (affinity mask 0x0F)
  - Skymont LP-E cores (affinity mask 0xF0)
Synthesizes 20 spoken device-commander confirmations and measures:
  - Audio duration generated vs. synthesis latency
  - Real-Time Factor (RTF = latency / audio_duration)
  - P-core vs. LP-E throughput and efficiency scaling
  - Monitored by mutop-ai for core placement, power, and thermals.
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
    if os.name == "nt":
        try:
            kernel32 = ctypes.windll.kernel32
            handle = kernel32.GetCurrentProcess()
            kernel32.SetProcessAffinityMask(handle, ctypes.c_size_t(mask))
        except Exception as e:
            print(f"Warning: Failed to set CPU affinity mask 0x{mask:X}: {e}", file=sys.stderr)


def reset_cpu_affinity() -> None:
    set_cpu_affinity(0xFF)


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


def text_to_synthetic_phoneme_ids(text: str, phoneme_id_map: dict) -> list:
    """Basic phoneme ID mapper fallback if piper-phonemize is not available."""
    # Maps basic ASCII chars to phoneme IDs, padded with inter-token blanks (id 0)
    res = [0]
    for char in text.lower():
        val = phoneme_id_map.get(char, [1])[0]
        res.append(val)
        res.append(0)
    return res


def run_scenario_1f(models_dir: str, mark_file: Optional[str] = None, log_file: Optional[str] = None):
    writer = MarkerWriter(mark_file) if mark_file else None
    results = {}

    import onnxruntime as ort

    onnx_path = os.path.join(models_dir, "tts", "piper", "en_GB-alan-medium.onnx")
    json_path = os.path.join(models_dir, "tts", "piper", "en_GB-alan-medium.onnx.json")

    if not os.path.exists(onnx_path):
        # Try alba fallback
        onnx_path = os.path.join(models_dir, "tts", "piper", "en_GB-alba-medium.onnx")
        json_path = os.path.join(models_dir, "tts", "piper", "en_GB-alba-medium.onnx.json")

    if not os.path.exists(onnx_path):
        print(f"Error: Piper ONNX model not found at {onnx_path}")
        return results

    with open(json_path, "r", encoding="utf-8") as f:
        model_config = json.load(f)

    phoneme_id_map = model_config.get("phoneme_id_map", {})
    sample_rate = model_config.get("audio", {}).get("sample_rate", 22050)

    sess_opts = ort.SessionOptions()
    sess_opts.intra_op_num_threads = 4
    sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

    print(f"\n================================================================================")
    print(f"  Loading Piper Neural TTS ONNX session ({os.path.basename(onnx_path)})...")
    print(f"================================================================================")
    session = ort.InferenceSession(onnx_path, sess_opts, providers=["CPUExecutionProvider"])

    # Warmup
    reset_cpu_affinity()
    dummy_input = np.array([[0, 10, 0, 20, 0]], dtype=np.int64)
    dummy_lengths = np.array([5], dtype=np.int64)
    dummy_scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)
    _ = session.run(None, {"input": dummy_input, "input_lengths": dummy_lengths, "scales": dummy_scales})

    core_configs = [
        ("cpu_p", 0x0F, "Lion Cove P-cores (C0..C3)"),
        ("cpu_lpe", 0xF0, "Skymont LP-E cores (C4..C7)"),
    ]

    for core_name, affinity_mask, core_desc in core_configs:
        phase_label = f"1F_piper_{core_name}"
        if writer:
            writer.set_marker(phase_label)

        print(f"\nSynthesizing 20 confirmations on {core_desc} (affinity 0x{affinity_mask:02X})...")
        set_cpu_affinity(affinity_mask)
        try:
            total_audio_sec = 0.0
            total_synth_sec = 0.0
            phrase_records = []

            for idx, phrase in enumerate(CONFIRMATION_PHRASES, 1):
                p_ids = text_to_synthetic_phoneme_ids(phrase, phoneme_id_map)
                input_tensor = np.array([p_ids], dtype=np.int64)
                lengths_tensor = np.array([len(p_ids)], dtype=np.int64)

                t0 = time.perf_counter()
                audio = session.run(None, {
                    "input": input_tensor,
                    "input_lengths": lengths_tensor,
                    "scales": dummy_scales
                })[0]
                t1 = time.perf_counter()

                synth_dur = t1 - t0
                num_audio_samples = audio.shape[-1]
                audio_dur = num_audio_samples / float(sample_rate)
                rtf = synth_dur / audio_dur if audio_dur > 0 else 0.0

                total_audio_sec += audio_dur
                total_synth_sec += synth_dur

                phrase_records.append({
                    "phrase": phrase,
                    "audio_dur_sec": audio_dur,
                    "synth_dur_sec": synth_dur,
                    "rtf": rtf
                })
                print(f"  [{idx:02d}/20] {phrase:<45} | Audio: {audio_dur:.2f}s | Synth: {synth_dur*1000:.1f}ms | RTF: {rtf:.3f}")

            overall_rtf = total_synth_sec / total_audio_sec if total_audio_sec > 0 else 0.0
            speed_mult = 1.0 / overall_rtf if overall_rtf > 0 else 0.0

            print(f"\nSummary for {core_desc}:")
            print(f"  Total Audio Generated : {total_audio_sec:.2f} s")
            print(f"  Total Synthesis Time  : {total_synth_sec:.2f} s")
            print(f"  Overall RTF           : {overall_rtf:.3f} ({speed_mult:.1f}x real-time)")

            results[f"Piper_{core_name}"] = {
                "core_target": core_desc,
                "total_phrases": len(CONFIRMATION_PHRASES),
                "total_audio_sec": total_audio_sec,
                "total_synth_sec": total_synth_sec,
                "overall_rtf": overall_rtf,
                "throughput_x_realtime": speed_mult,
                "phrases": phrase_records
            }
        finally:
            reset_cpu_affinity()

    if writer:
        writer.set_marker("1F_done")

    if log_file:
        os.makedirs(os.path.dirname(os.path.abspath(log_file)), exist_ok=True)
        with open(log_file, "w", encoding="utf-8") as f:
            f.write(json.dumps(results, indent=2))
        print(f"\nSaved Scenario 1F results to {log_file}")

    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--models-dir", default=r"C:\Users\peter\models")
    parser.add_argument("--mark-file", default=None)
    parser.add_argument("--log-file", default=r"C:\develop\mutop\logs\scenario_1f_piper_tts.log")
    args = parser.parse_args()
    run_scenario_1f(args.models_dir, args.mark_file, args.log_file)
