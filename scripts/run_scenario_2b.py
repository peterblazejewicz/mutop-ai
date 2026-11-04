#!/usr/bin/env python3
"""
run_scenario_2b.py - Scenario 2B: Sustained Tri-Accelerator Stress & UMA Contention Test (60s).

Drives all 3 hardware engines under continuous concurrent load for 60 seconds:
  - GPU (Arc 140V): llama-server serving Gemma 4 E4B (continuous PP512 + TG128 loop)
  - NPU (AI Boost 4): Continuous micro-batch embeddings (EmbeddingGemma-300M [4, 512])
                     + FunctionGemma-270M intent classification loop
  - CPU (Lion Cove / Skymont): Continuous Moonshine Base STT (10s audio) + Piper TTS across 8 cores

Phase Markers:
  - 2B_ramp_0_20s       (0s - 20s)
  - 2B_sustained_20_40s  (20s - 40s)
  - 2B_saturation_40_60s (40s - 60s)
  - 2B_cooldown          (60s - 70s, 10s cool-off)

Monitored by:
  mutop-ai -s 200 --mark-file ... --proc llama-server,python --threads --summary
"""

import concurrent.futures
import json
import os
import re
import subprocess
import sys
import threading
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
    with urllib.request.urlopen(req, timeout=90.0) as resp:
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

    # Parse interval markers if present
    stats["intervals"] = []
    int_matches = re.findall(r"\[\s*(\d+)\] Phase\s*:\s*(\S+)\s*\(([\d\.]+)s\)\s*Power med\s*([\d\.]+)W\s*/\s*max\s*([\d\.]+)W.*?(?:Fan\s*med\s*(\d+)\s*/\s*max\s*(\d+)\s*RPM|Fan\s*max\s*(\d+)\s*RPM)?", content)
    for m in int_matches:
        idx, phase_name, dur, p_med, p_max, f_med, f_max1, f_max2 = m
        fan_max = int(f_max1) if f_max1 else (int(f_max2) if f_max2 else 0)
        fan_med = int(f_med) if f_med else fan_max
        stats["intervals"].append({
            "phase": phase_name,
            "duration_sec": float(dur),
            "power_med_w": float(p_med),
            "power_max_w": float(p_max),
            "fan_med_rpm": fan_med,
            "fan_max_rpm": fan_max,
        })

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
    scenario_id = "2B_tri_accelerator_stress_60s"
    log_file = os.path.join(LOG_DIR, "scenario_2b_sustained_stress.log")

    print("================================================================================")
    print(">>> Scenario 2B: 60-Second Sustained Tri-Accelerator Stress & UMA Contention")
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
    stop_event = threading.Event()

    gpu_stats = {"queries": 0, "total_tokens": 0, "durations": []}
    npu_stats = {"batches_b4_512": 0, "fg_queries": 0, "durations": []}
    cpu_stats = {"stt_queries": 0, "tts_queries": 0, "stt_durations": [], "tts_durations": []}

    try:
        # 3. Start llama-server on Intel Arc 140V GPU (Gemma 4 E4B Q4_K_M)
        e4b_path = os.path.join(MODELS_ROOT, "slm", "gemma-4-E4B-it-Q4_K_M.gguf")
        server_cmd = [VULKAN_SERVER, "-m", e4b_path, "-ngl", "99", "--port", "8080", "--slots", "-c", "2048"]
        server_proc = subprocess.Popen(server_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print("    Waiting for llama-server (Arc 140V GPU) on port 8080...")
        if not wait_for_server(timeout_sec=45.0):
            print("Error: llama-server failed to initialize.")
            return

        # 4. Initialize NPU models (EmbeddingGemma [4, 512] + FunctionGemma-270M)
        print("    Loading EmbeddingGemma-300M ([4, 512]) onto Intel AI Boost NPU 4...")
        embed_xml = os.path.join(MODELS_ROOT, "embedding", "EmbeddingGemma-300M", "openvino_model.xml")
        cache_dir = os.path.join(MODELS_ROOT, ".ov_cache")
        ov_core = ov.Core()
        ov_core.set_property("NPU", {"CACHE_DIR": cache_dir})
        embed_model = ov_core.read_model(embed_xml)
        embed_model.reshape({
            "input_ids": [4, 512],
            "attention_mask": [4, 512],
            "position_ids": [4, 512]
        })
        compiled_embed = ov_core.compile_model(embed_model, "NPU")
        embed_req = compiled_embed.create_infer_request()
        dummy_512_b4 = {
            "input_ids": np.ones((4, 512), dtype=np.int64),
            "attention_mask": np.ones((4, 512), dtype=np.int64),
            "position_ids": np.tile(np.arange(512, dtype=np.int64), (4, 1))
        }
        embed_req.infer(dummy_512_b4)

        print("    Loading FunctionGemma-270M onto Intel AI Boost NPU 4...")
        fg_dir = os.path.join(MODELS_ROOT, "llm", "FunctionGemma-270M-INT8")
        npu_pipe = ov_genai.LLMPipeline(fg_dir, "NPU", CACHE_DIR=cache_dir)
        _ = npu_pipe.generate("warmup volume up", max_new_tokens=16)

        # 5. Initialize CPU models (Moonshine Base STT + Piper TTS)
        print("    Loading Moonshine Base & Piper TTS on CPU...")
        stt_onnx = os.path.join(MODELS_ROOT, "stt", "moonshine", "base", "onnx", "encoder_model.onnx")
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

        # 10s audio for STT
        audio_dur = 10.0
        t_arr = np.linspace(0, audio_dur, int(audio_dur * 16000), dtype=np.float32)
        test_audio = (0.5 * np.sin(2 * np.pi * 440 * t_arr)).reshape(1, -1)

        # 45-word diagnostic confirmation for TTS
        tts_long_text = "System diagnostic active. Intel Core Ultra 7 256V package operating nominal at 24 watts. Intel Arc 140V Xe2 graphics core running at maximum boost frequency. Intel AI Boost neural processing unit actively processing batch document embeddings across sixteen gigabytes shared memory."
        p_ids = text_to_phonemes(tts_long_text, phoneme_map)
        tts_inp = np.array([p_ids], dtype=np.int64)
        tts_lens = np.array([len(p_ids)], dtype=np.int64)

        # 6. Worker Thread Functions
        def worker_gpu():
            # ~500 token prompt to trigger PP512 + TG128
            long_prompt = "You are Device Commander running on Intel Core Ultra 7. " * 30 + "Explain system memory architecture and audio DSP pipelines in detail:"
            while not stop_event.is_set():
                try:
                    res = query_completion(long_prompt, n_predict=128)
                    gpu_stats["queries"] += 1
                    gpu_stats["total_tokens"] += res["predicted_n"]
                    gpu_stats["durations"].append(res["duration_sec"])
                except Exception as e:
                    if not stop_event.is_set():
                        print(f"    [GPU Worker Error]: {e}")
                    time.sleep(0.5)

        def worker_npu():
            fg_prompts = [
                "switch to noise cancelling mode",
                "set volume to eighty percent",
                "enable hear through mode",
                "run hardware diagnostic"
            ]
            p_idx = 0
            while not stop_event.is_set():
                try:
                    t0 = time.perf_counter()
                    # 1. Micro-batch embeddings [4, 512]
                    embed_req.infer(dummy_512_b4)
                    npu_stats["batches_b4_512"] += 1

                    # 2. FunctionGemma query
                    _ = npu_pipe.generate(fg_prompts[p_idx % len(fg_prompts)], max_new_tokens=24)
                    p_idx += 1
                    npu_stats["fg_queries"] += 1

                    t1 = time.perf_counter()
                    npu_stats["durations"].append(t1 - t0)
                except Exception as e:
                    if not stop_event.is_set():
                        print(f"    [NPU Worker Error]: {e}")
                    time.sleep(0.1)

        def worker_cpu():
            while not stop_event.is_set():
                try:
                    # STT
                    t0 = time.perf_counter()
                    _ = stt_sess.run(None, {stt_in_name: test_audio})
                    t1 = time.perf_counter()
                    cpu_stats["stt_queries"] += 1
                    cpu_stats["stt_durations"].append(t1 - t0)

                    # TTS
                    t2 = time.perf_counter()
                    _ = tts_sess.run(None, {"input": tts_inp, "input_lengths": tts_lens, "scales": tts_scales})
                    t3 = time.perf_counter()
                    cpu_stats["tts_queries"] += 1
                    cpu_stats["tts_durations"].append(t3 - t2)
                except Exception as e:
                    if not stop_event.is_set():
                        print(f"    [CPU Worker Error]: {e}")
                    time.sleep(0.1)

        print("\n>>> Launching Simultaneous Multi-Accelerator Workers...")
        t_gpu = threading.Thread(target=worker_gpu, name="Worker-GPU")
        t_npu = threading.Thread(target=worker_npu, name="Worker-NPU")
        t_cpu = threading.Thread(target=worker_cpu, name="Worker-CPU")

        t_gpu.daemon = True
        t_npu.daemon = True
        t_cpu.daemon = True

        # Phase 1: 2B_ramp_0_20s
        print(">>> Phase: 2B_ramp_0_20s (Seconds 0 - 20)...")
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write("2B_ramp_0_20s")

        t_gpu.start()
        t_npu.start()
        t_cpu.start()

        time.sleep(20.0)

        # Phase 2: 2B_sustained_20_40s
        print(">>> Phase: 2B_sustained_20_40s (Seconds 20 - 40)...")
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write("2B_sustained_20_40s")

        time.sleep(20.0)

        # Phase 3: 2B_saturation_40_60s
        print(">>> Phase: 2B_saturation_40_60s (Seconds 40 - 60)...")
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write("2B_saturation_40_60s")

        time.sleep(20.0)

        # Stop workloads
        print(">>> Stopping workloads. Entering 10s cooldown...")
        stop_event.set()

        t_gpu.join(timeout=10.0)
        t_npu.join(timeout=10.0)
        t_cpu.join(timeout=10.0)

        # Phase 4: 2B_cooldown (10s)
        print(">>> Phase: 2B_cooldown (Seconds 60 - 70)...")
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write("2B_cooldown")

        time.sleep(10.0)

    finally:
        stop_event.set()
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
        "scenario": "2B_sustained_tri_accelerator_stress_60s",
        "workload_stats": {
            "gpu": {
                "model": "Gemma 4 E4B Q4_K_M (Vulkan)",
                "completed_queries": gpu_stats["queries"],
                "total_generated_tokens": gpu_stats["total_tokens"],
                "avg_query_sec": round(float(np.mean(gpu_stats["durations"])), 3) if gpu_stats["durations"] else 0.0,
            },
            "npu": {
                "models": "EmbeddingGemma-300M [4, 512] + FunctionGemma-270M INT8",
                "completed_batches_b4_512": npu_stats["batches_b4_512"],
                "total_embedded_docs": npu_stats["batches_b4_512"] * 4,
                "completed_fg_intent_queries": npu_stats["fg_queries"],
                "avg_loop_ms": round(float(np.mean(npu_stats["durations"])) * 1000.0, 1) if npu_stats["durations"] else 0.0,
            },
            "cpu": {
                "models": "Moonshine Base STT + Piper en_GB-alan-medium TTS",
                "completed_stt_transcriptions_10s": cpu_stats["stt_queries"],
                "avg_stt_sec": round(float(np.mean(cpu_stats["stt_durations"])), 3) if cpu_stats["stt_durations"] else 0.0,
                "completed_tts_syntheses_45words": cpu_stats["tts_queries"],
                "avg_tts_sec": round(float(np.mean(cpu_stats["tts_durations"])), 3) if cpu_stats["tts_durations"] else 0.0,
            }
        },
        "telemetry": summary_metrics,
        "log_file": log_file
    }

    summary_json_path = os.path.join(LOG_DIR, "scenario_2b_summary.json")
    with open(summary_json_path, "w", encoding="utf-8") as f:
        json.dump(final_output, f, indent=2)

    print(f"\n================================================================================")
    print(f"Scenario 2B Complete. 60s Sustained Stress Summary:")
    print(f"  GPU Queries   : {gpu_stats['queries']} queries ({gpu_stats['total_tokens']} tokens, Gemma 4 E4B)")
    print(f"  NPU Batches   : {npu_stats['batches_b4_512']} micro-batches ({npu_stats['batches_b4_512']*4} docs [512 tok]) + {npu_stats['fg_queries']} FG intents")
    print(f"  CPU STT / TTS : {cpu_stats['stt_queries']} x 10s STT | {cpu_stats['tts_queries']} x 45-word TTS syntheses")
    print(f"  Telemetry     : Power median {summary_metrics.get('power_median_w', 'N/A')} W (max {summary_metrics.get('power_max_w', 'N/A')} W)")
    print(f"                  NPU/GPU Act: {summary_metrics.get('npu_gpu_active_share_pct', 'N/A')}% | Fan: {summary_metrics.get('fan_median_rpm', 'N/A')} RPM (max {summary_metrics.get('fan_max_rpm', 'N/A')} RPM)")
    print(f"                  CPU DTS Temp: {summary_metrics.get('cpu_temp_median_c', 'N/A')} C (max {summary_metrics.get('cpu_temp_max_c', 'N/A')} C)")
    print(f"================================================================================")


if __name__ == "__main__":
    main()
