#!/usr/bin/env python3
"""
run_scenario_2a.py - Scenario 2A: Interactive Voice Turn Simulation (Tri-Engine Concurrency).

Executes 10 consecutive composite turns where all 3 compute engines operate simultaneously:
  - GPU (Arc 140V): llama-server serving Gemma 4 E2B generating 128 tokens
  - NPU (AI Boost 4): FunctionGemma-270M tool intent classification + Embedding lookup
  - CPU (Lion Cove / Skymont): Moonshine Tiny STT + Piper VITS TTS synthesis

Monitored by mutop-ai -s 200 --mark-file ... --proc llama-server,python --threads --summary.
"""

import concurrent.futures
import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from typing import Any, Dict, List
import numpy as np
import onnxruntime as ort
import openvino as ov
import openvino_genai as ov_genai


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MARK_FILE = r"C:\develop\mutop\scripts\mutop_markers.txt"
MUTOP_EXE = r"C:\develop\mutop\mutop-ai.exe"
LOG_DIR = r"C:\develop\mutop\logs"
MODELS_ROOT = r"C:\Users\peter\models"
VULKAN_SERVER = r"C:\tools\llama.cpp-vulkan\llama-server.exe"


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


def wait_for_server(url: str = "http://127.0.0.1:8080", timeout_sec: float = 40.0) -> bool:
    start = time.time()
    while time.time() - start < timeout_sec:
        try:
            req = urllib.request.Request(f"{url}/health")
            with urllib.request.urlopen(req, timeout=1.0) as resp:
                if resp.status == 200:
                    return True
        except Exception:
            time.sleep(0.5)
    return False


def query_completion(prompt: str, n_predict: int = 128) -> Dict[str, Any]:
    url = "http://127.0.0.1:8080/completion"
    payload = json.dumps({
        "prompt": prompt,
        "n_predict": n_predict,
        "temperature": 0.0,
        "stream": False,
        "cache_prompt": False
    }).encode("utf-8")
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=60.0) as resp:
        t1 = time.perf_counter()
        data = json.loads(resp.read().decode("utf-8"))
    timings = data.get("timings", {})
    return {
        "duration_sec": t1 - t0,
        "predicted_tok_s": timings.get("predicted_per_second", 0.0),
        "predicted_n": timings.get("predicted_n", n_predict),
    }


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

    m_gpu = re.search(r"GPU Active Duty \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_gpu:
        stats["gpu_median_pct"] = float(m_gpu.group(2))
        stats["gpu_max_pct"] = float(m_gpu.group(4))

    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))
        stats["cpu_temp_max_c"] = float(m_temp.group(4))

    m_act = re.search(r"Active Intel AI Boost NPU & Arc 140V GPU Share:\s+([\d\.]+)%", content)
    if m_act:
        stats["npu_gpu_active_share_pct"] = float(m_act.group(1))

    return stats


def text_to_phonemes(text: str, phoneme_map: dict) -> list:
    res = [0]
    for char in text.lower():
        val = phoneme_map.get(char, [1])[0]
        res.append(val)
        res.append(0)
    return res


def main():
    os.makedirs(LOG_DIR, exist_ok=True)
    scenario_id = "2A_voice_turn_composite"
    log_file = os.path.join(LOG_DIR, "scenario_2a_voice_turn.log")

    print("================================================================================")
    print(">>> Scenario 2A: Interactive Voice Turn Simulation (Tri-Engine Concurrency)")
    print("================================================================================")

    acquire_lock(scenario_id)

    # 1. Reset mark file
    with open(MARK_FILE, "w", encoding="utf-8") as f:
        f.write("")

    # 2. Launch mutop-ai collector
    mutop_cmd = [
        MUTOP_EXE,
        "-s", "200",
        "--mark-file", MARK_FILE,
        "--proc", "llama-server,python",
        "--threads",
        "--summary"
    ]
    with open(log_file, "w", encoding="utf-8") as out_f:
        mutop_proc = subprocess.Popen(mutop_cmd, stdout=out_f, stderr=subprocess.STDOUT)

    time.sleep(0.5)

    server_proc = None
    all_turns_data: List[Dict[str, Any]] = []

    try:
        # 3. Start llama-server on Intel Arc 140V GPU (Gemma 4 E2B)
        e2b_path = os.path.join(MODELS_ROOT, "slm", "gemma-4-E2B-it-Q4_K_M.gguf")
        server_cmd = [VULKAN_SERVER, "-m", e2b_path, "-ngl", "99", "--port", "8080", "--slots", "-c", "2048"]
        server_proc = subprocess.Popen(server_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print("    Waiting for llama-server (Arc 140V GPU) on port 8080...")
        if not wait_for_server(timeout_sec=40.0):
            print("Error: llama-server failed to initialize.")
            return

        # 4. Initialize FunctionGemma-270M and EmbeddingGemma-300M on NPU
        print("    Loading FunctionGemma-270M onto Intel AI Boost NPU 4...")
        fg_dir = os.path.join(MODELS_ROOT, "llm", "FunctionGemma-270M-INT8")
        cache_dir = os.path.join(MODELS_ROOT, ".ov_cache")
        npu_pipe = ov_genai.LLMPipeline(fg_dir, "NPU", CACHE_DIR=cache_dir)
        _ = npu_pipe.generate("warmup volume up", max_new_tokens=16)

        print("    Loading EmbeddingGemma-300M onto Intel AI Boost NPU 4...")
        embed_xml = os.path.join(MODELS_ROOT, "embedding", "EmbeddingGemma-300M", "openvino_model.xml")
        ov_core = ov.Core()
        ov_core.set_property("NPU", {"CACHE_DIR": cache_dir})
        embed_model = ov_core.read_model(embed_xml)
        embed_model.reshape({
            "input_ids": [1, 128],
            "attention_mask": [1, 128],
            "position_ids": [1, 128]
        })
        compiled_embed = ov_core.compile_model(embed_model, "NPU")
        embed_req = compiled_embed.create_infer_request()
        dummy_128 = {
            "input_ids": np.ones((1, 128), dtype=np.int64),
            "attention_mask": np.ones((1, 128), dtype=np.int64),
            "position_ids": np.arange(128, dtype=np.int64).reshape((1, 128))
        }
        embed_req.infer(dummy_128)

        # 5. Initialize Moonshine Tiny & Piper TTS on CPU
        print("    Loading Moonshine Tiny & Piper TTS on CPU...")
        stt_onnx = os.path.join(MODELS_ROOT, "stt", "moonshine", "tiny", "onnx", "encoder_model.onnx")
        tts_onnx = os.path.join(MODELS_ROOT, "tts", "piper", "en_GB-alan-medium.onnx")
        tts_json = os.path.join(MODELS_ROOT, "tts", "piper", "en_GB-alan-medium.onnx.json")

        sess_opts = ort.SessionOptions()
        sess_opts.intra_op_num_threads = 4
        sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

        stt_sess = ort.InferenceSession(stt_onnx, sess_opts, providers=["CPUExecutionProvider"])
        stt_in_name = stt_sess.get_inputs()[0].name

        tts_sess = ort.InferenceSession(tts_onnx, sess_opts, providers=["CPUExecutionProvider"])
        with open(tts_json, "r", encoding="utf-8") as f:
            tts_cfg = json.load(f)
        phoneme_map = tts_cfg.get("phoneme_id_map", {})
        tts_scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)

        # Synthetic 10s audio for STT
        audio_dur = 10.0
        t_arr = np.linspace(0, audio_dur, int(audio_dur * 16000), dtype=np.float32)
        test_audio = (0.5 * np.sin(2 * np.pi * 440 * t_arr)).reshape(1, -1)

        print("\n>>> Executing 10 Consecutive Tri-Engine Composite Voice Turns...")

        turns_prompts = [
            ("switch to noise cancelling mode", "Noise cancelling enabled. Sidetone level four."),
            ("set volume to eighty percent", "Volume adjusted to eighty percent."),
            ("enable hear through mode", "HearThrough mode active."),
            ("what is my current battery level", "Battery level is eighty percent remaining."),
            ("mute microphone", "Microphone muted."),
            ("unmute microphone", "Microphone unmuted."),
            ("apply speech clarity equalizer", "Equaliser profile speech clarity applied."),
            ("set sleep timer to thirty minutes", "Sleep timer set to thirty minutes."),
            ("turn busylight on", "Busylight active."),
            ("run hardware diagnostic", "System healthy. CPU package temperature is 48 degrees."),
        ]

        for turn_idx, (turn_user_req, turn_tts_phrase) in enumerate(turns_prompts, 1):
            marker_label = f"2A_turn_{turn_idx:02d}"
            with open(MARK_FILE, "w", encoding="utf-8") as f:
                f.write(marker_label)
            time.sleep(0.25)

            turn_start = time.perf_counter()

            # Concurrent execution definitions
            def task_gpu():
                prompt = f"The user wearing smart headset says: '{turn_user_req}'. Provide concise smart response: "
                return query_completion(prompt, n_predict=128)

            def task_npu():
                t0 = time.perf_counter()
                res = npu_pipe.generate(turn_user_req, max_new_tokens=24)
                t1 = time.perf_counter()
                embed_req.infer(dummy_128)
                t2 = time.perf_counter()
                return {
                    "duration_sec": t2 - t0,
                    "fg_ms": (t1 - t0) * 1000.0,
                    "embed_ms": (t2 - t1) * 1000.0,
                    "text": res
                }

            def task_cpu():
                # STT phase
                t0 = time.perf_counter()
                _ = stt_sess.run(None, {stt_in_name: test_audio})
                t1 = time.perf_counter()
                stt_dur = t1 - t0

                # TTS phase
                p_ids = text_to_phonemes(turn_tts_phrase, phoneme_map)
                inp = np.array([p_ids], dtype=np.int64)
                lens = np.array([len(p_ids)], dtype=np.int64)
                t2 = time.perf_counter()
                _ = tts_sess.run(None, {"input": inp, "input_lengths": lens, "scales": tts_scales})
                t3 = time.perf_counter()
                tts_dur = t3 - t2

                return {"stt_dur_ms": stt_dur * 1000.0, "tts_dur_ms": tts_dur * 1000.0}

            with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
                f_gpu = executor.submit(task_gpu)
                f_npu = executor.submit(task_npu)
                f_cpu = executor.submit(task_cpu)

                res_gpu = f_gpu.result()
                res_npu = f_npu.result()
                res_cpu = f_cpu.result()

            turn_total_sec = time.perf_counter() - turn_start

            turn_stat = {
                "turn": turn_idx,
                "total_turn_sec": round(turn_total_sec, 3),
                "gpu_tok_s": round(res_gpu.get("predicted_tok_s", 0.0), 2),
                "gpu_latency_sec": round(res_gpu.get("duration_sec", 0.0), 3),
                "npu_latency_ms": round(res_npu.get("duration_sec", 0.0) * 1000.0, 1),
                "npu_intent_ms": round(res_npu.get("fg_ms", 0.0), 1),
                "npu_embed_ms": round(res_npu.get("embed_ms", 0.0), 1),
                "cpu_stt_ms": round(res_cpu.get("stt_dur_ms", 0.0), 1),
                "cpu_tts_ms": round(res_cpu.get("tts_dur_ms", 0.0), 1),
            }
            all_turns_data.append(turn_stat)

            print(f"  [{turn_idx:02d}/10] Total: {turn_total_sec:.2f}s | GPU: {turn_stat['gpu_tok_s']:>5.1f} tok/s | NPU: {turn_stat['npu_latency_ms']:>5.1f} ms (FG: {turn_stat['npu_intent_ms']:>4.1f}ms, Emb: {turn_stat['npu_embed_ms']:>4.1f}ms) | STT: {turn_stat['cpu_stt_ms']:>4.1f} ms | TTS: {turn_stat['cpu_tts_ms']:>5.1f} ms")

    finally:
        if server_proc:
            server_proc.terminate()
            try:
                server_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server_proc.kill()

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
    final_output = {
        "scenario": "2A_interactive_voice_turn_tri_engine",
        "turns": all_turns_data,
        "averages": {
            "avg_turn_sec": round(float(np.mean([t["total_turn_sec"] for t in all_turns_data])), 3),
            "avg_gpu_tok_s": round(float(np.mean([t["gpu_tok_s"] for t in all_turns_data])), 2),
            "avg_npu_ms": round(float(np.mean([t["npu_latency_ms"] for t in all_turns_data])), 1),
            "avg_npu_intent_ms": round(float(np.mean([t["npu_intent_ms"] for t in all_turns_data])), 1),
            "avg_npu_embed_ms": round(float(np.mean([t["npu_embed_ms"] for t in all_turns_data])), 1),
            "avg_stt_ms": round(float(np.mean([t["cpu_stt_ms"] for t in all_turns_data])), 1),
            "avg_tts_ms": round(float(np.mean([t["cpu_tts_ms"] for t in all_turns_data])), 1),
        },
        "telemetry": summary_metrics,
        "log_file": log_file
    }

    summary_json_path = os.path.join(LOG_DIR, "scenario_2a_summary.json")
    with open(summary_json_path, "w", encoding="utf-8") as f:
        json.dump(final_output, f, indent=2)

    print(f"\n================================================================================")
    print(f"Scenario 2A Complete. Averages across 10 concurrent turns:")
    print(f"  Turn Duration : {final_output['averages']['avg_turn_sec']} s")
    print(f"  GPU Decode    : {final_output['averages']['avg_gpu_tok_s']} tok/s (Gemma 4 E2B)")
    print(f"  NPU Tool-Call : {final_output['averages']['avg_npu_ms']} ms (FunctionGemma-270M)")
    print(f"  CPU STT       : {final_output['averages']['avg_stt_ms']} ms (Moonshine Tiny)")
    print(f"  CPU TTS       : {final_output['averages']['avg_tts_ms']} ms (Piper Alan)")
    print(f"  Telemetry     : Power median {summary_metrics.get('power_median_w', 'N/A')} W (max {summary_metrics.get('power_max_w', 'N/A')} W)")
    print(f"                  NPU/GPU Act: {summary_metrics.get('npu_gpu_active_share_pct', 'N/A')}% | Fan: {summary_metrics.get('fan_median_rpm', 'N/A')} RPM | Temp: {summary_metrics.get('cpu_temp_median_c', 'N/A')} C")
    print(f"================================================================================")


if __name__ == "__main__":
    main()
