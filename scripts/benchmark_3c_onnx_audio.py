#!/usr/bin/env python3
"""
benchmark_3c_onnx_audio.py - Scenario 3C: STT/TTS ONNX Startup, Voice Preloading & Tiered Audio Cache.

Evaluates on Intel Core Ultra 7 256V CPU (Skymont LP-E & Lion Cove P-cores):
  1. ONNX Runtime Offline Graph Optimization Serialization & Arena Warmup (Moonshine & Piper)
  2. Piper Voice Preloading & Tiered Synthesis Latency:
     - Mode 1: Cold process/session spawn + synthesis
     - Mode 2: Warm preloaded InferenceSession + preloaded phoneme map
     - Mode 3: Pre-synthesized RAM PCM Reflex Cache lookup (0 ms synthesis / <10 ms playback)
     - Mode 4: Sentence-chunked streaming TTS overlapped with SLM token generation (TTFA)
"""

import json
import os
import subprocess
import sys
import threading
import time
from typing import Any, Dict, List
import numpy as np
import onnxruntime as ort


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MODELS_ROOT = r"C:\Users\peter\models"
MOONSHINE_TINY_ONNX = os.path.join(MODELS_ROOT, "stt", "moonshine", "tiny", "onnx", "encoder_model.onnx")
MOONSHINE_BASE_ONNX = os.path.join(MODELS_ROOT, "stt", "moonshine", "base", "onnx", "encoder_model.onnx")
PIPER_ONNX = os.path.join(MODELS_ROOT, "tts", "piper", "en_GB-alan-medium.onnx")
PIPER_JSON = os.path.join(MODELS_ROOT, "tts", "piper", "en_GB-alan-medium.onnx.json")
VULKAN_SERVER = r"C:\tools\llama.cpp-vulkan\llama-server.exe"
E2B_PATH = os.path.join(MODELS_ROOT, "slm", "gemma-4-E2B-it-Q4_K_M.gguf")


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


def text_to_phonemes(text: str, phoneme_map: dict) -> list:
    res = [0]
    for char in text.lower():
        val = phoneme_map.get(char, [1])[0]
        res.append(val)
        res.append(0)
    return res


def benchmark_onnx_graph_serialization_and_arena() -> Dict[str, Any]:
    print("\n--- [3C] 1. ONNX Offline Graph Serialization & Arena Warmup ---")
    results = {}

    targets = [
        ("Moonshine_Tiny", MOONSHINE_TINY_ONNX),
        ("Moonshine_Base", MOONSHINE_BASE_ONNX),
        ("Piper_Alan", PIPER_ONNX)
    ]

    for name, raw_model_path in targets:
        print(f"\n  Evaluating {name}...")
        opt_model_path = raw_model_path.replace(".onnx", "_opt.onnx")

        # 1. Cold Session Init (Unoptimized model + Online fusion with ORT_ENABLE_ALL)
        opts_cold = ort.SessionOptions()
        opts_cold.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        opts_cold.optimized_model_filepath = opt_model_path
        opts_cold.intra_op_num_threads = 4

        t0 = time.perf_counter()
        sess_cold = ort.InferenceSession(raw_model_path, opts_cold, providers=["CPUExecutionProvider"])
        t1 = time.perf_counter()
        cold_init_ms = (t1 - t0) * 1000.0

        # Run Turn 1 (cold arena allocation)
        if "Moonshine" in name:
            in_name = sess_cold.get_inputs()[0].name
            dummy_10s = np.zeros((1, 16000 * 10), dtype=np.float32)
            t2 = time.perf_counter()
            _ = sess_cold.run(None, {in_name: dummy_10s})
            t3 = time.perf_counter()
            cold_turn1_infer_ms = (t3 - t2) * 1000.0
        else:
            p_ids = [0, 1, 0, 2, 0]
            inp = np.array([p_ids], dtype=np.int64)
            lens = np.array([len(p_ids)], dtype=np.int64)
            scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)
            t2 = time.perf_counter()
            _ = sess_cold.run(None, {"input": inp, "input_lengths": lens, "scales": scales})
            t3 = time.perf_counter()
            cold_turn1_infer_ms = (t3 - t2) * 1000.0

        del sess_cold

        # 2. Warm Session Init from pre-serialized optimized model
        opts_warm = ort.SessionOptions()
        opts_warm.graph_optimization_level = ort.GraphOptimizationLevel.ORT_DISABLE_ALL
        opts_warm.intra_op_num_threads = 4
        opts_warm.enable_cpu_mem_arena = True
        opts_warm.enable_mem_pattern = True

        t4 = time.perf_counter()
        sess_warm = ort.InferenceSession(opt_model_path, opts_warm, providers=["CPUExecutionProvider"])
        t5 = time.perf_counter()
        warm_init_ms = (t5 - t4) * 1000.0

        # Pre-warm 1 frame
        if "Moonshine" in name:
            in_name = sess_warm.get_inputs()[0].name
            dummy_1frame = np.zeros((1, 16000), dtype=np.float32)
            _ = sess_warm.run(None, {in_name: dummy_1frame})
            dummy_10s = np.zeros((1, 16000 * 10), dtype=np.float32)
            t6 = time.perf_counter()
            _ = sess_warm.run(None, {in_name: dummy_10s})
            t7 = time.perf_counter()
            warm_turn2_infer_ms = (t7 - t6) * 1000.0
        else:
            p_ids = [0, 1, 0]
            _ = sess_warm.run(None, {"input": np.array([p_ids], dtype=np.int64), "input_lengths": np.array([len(p_ids)], dtype=np.int64), "scales": scales})
            t6 = time.perf_counter()
            _ = sess_warm.run(None, {"input": inp, "input_lengths": lens, "scales": scales})
            t7 = time.perf_counter()
            warm_turn2_infer_ms = (t7 - t6) * 1000.0

        del sess_warm

        results[name] = {
            "cold_init_ms": round(cold_init_ms, 1),
            "cold_turn1_infer_ms": round(cold_turn1_infer_ms, 1),
            "warm_init_opt_ms": round(warm_init_ms, 1),
            "warm_turn2_infer_ms": round(warm_turn2_infer_ms, 1),
            "session_init_speedup_x": round(cold_init_ms / max(warm_init_ms, 0.1), 2),
            "infer_arena_speedup_x": round(cold_turn1_infer_ms / max(warm_turn2_infer_ms, 0.1), 2),
        }

        print(f"    Cold Init: {results[name]['cold_init_ms']} ms | Warm Init: {results[name]['warm_init_opt_ms']} ms ({results[name]['session_init_speedup_x']}x faster)")
        print(f"    Cold Turn 1: {results[name]['cold_turn1_infer_ms']} ms | Warm Turn 2+: {results[name]['warm_turn2_infer_ms']} ms ({results[name]['infer_arena_speedup_x']}x faster)")

    return results


def benchmark_piper_tiered_synthesis_and_caching() -> Dict[str, Any]:
    print("\n--- [3C] 2. Piper Tiered Synthesis & Pre-Synthesized Audio Caching ---")
    with open(PIPER_JSON, "r", encoding="utf-8") as f:
        tts_cfg = json.load(f)
    phoneme_map = tts_cfg.get("phoneme_id_map", {})
    scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)

    sample_phrase = "Noise cancelling active. Sidetone level four."
    p_ids = text_to_phonemes(sample_phrase, phoneme_map)
    inp = np.array([p_ids], dtype=np.int64)
    lens = np.array([len(p_ids)], dtype=np.int64)

    # Mode 1: Cold process/session spawn + synthesize
    t0 = time.perf_counter()
    opts_cold = ort.SessionOptions()
    opts_cold.intra_op_num_threads = 4
    sess_cold = ort.InferenceSession(PIPER_ONNX, opts_cold, providers=["CPUExecutionProvider"])
    _ = sess_cold.run(None, {"input": inp, "input_lengths": lens, "scales": scales})
    t1 = time.perf_counter()
    mode1_cold_ms = (t1 - t0) * 1000.0
    del sess_cold

    # Mode 2: Warm preloaded session
    opts_warm = ort.SessionOptions()
    opts_warm.intra_op_num_threads = 4
    sess_warm = ort.InferenceSession(PIPER_ONNX, opts_warm, providers=["CPUExecutionProvider"])
    # warmup
    _ = sess_warm.run(None, {"input": inp, "input_lengths": lens, "scales": scales})

    latencies_mode2 = []
    for _ in range(5):
        t2 = time.perf_counter()
        audio = sess_warm.run(None, {"input": inp, "input_lengths": lens, "scales": scales})[0]
        t3 = time.perf_counter()
        latencies_mode2.append((t3 - t2) * 1000.0)
    mode2_warm_ms = float(np.mean(latencies_mode2))

    # Mode 3: Pre-synthesized RAM PCM Reflex Cache Lookup
    # Pre-render to byte array
    ram_pcm_cache: Dict[str, bytes] = {
        sample_phrase: audio.tobytes(),
        "Volume 80 percent": (audio * 0.8).tobytes(),
        "Microphone muted": (audio * 0.5).tobytes()
    }

    latencies_mode3 = []
    for _ in range(20):
        t4 = time.perf_counter()
        buf = ram_pcm_cache.get(sample_phrase)
        t5 = time.perf_counter()
        latencies_mode3.append((t5 - t4) * 1000.0)
    mode3_ram_cache_ms = float(np.mean(latencies_mode3))

    del sess_warm

    res = {
        "mode_1_cold_spawn_synthesis_ms": round(mode1_cold_ms, 1),
        "mode_2_warm_preloaded_session_ms": round(mode2_warm_ms, 1),
        "mode_3_ram_pcm_reflex_cache_lookup_ms": round(mode3_ram_cache_ms, 4),
        "reflex_cache_speedup_vs_cold_x": round(mode1_cold_ms / max(mode3_ram_cache_ms, 0.001), 1),
        "reflex_cache_speedup_vs_warm_x": round(mode2_warm_ms / max(mode3_ram_cache_ms, 0.001), 1),
    }

    print(f"  Mode 1 (Cold Spawn + Synthesize): {res['mode_1_cold_spawn_synthesis_ms']} ms")
    print(f"  Mode 2 (Warm Preloaded Session) : {res['mode_2_warm_preloaded_session_ms']} ms")
    print(f"  Mode 3 (RAM PCM Reflex Cache)   : {res['mode_3_ram_pcm_reflex_cache_lookup_ms']} ms ({res['reflex_cache_speedup_vs_warm_x']}x faster than warm)")

    return res


def benchmark_streaming_overlapped_tts() -> Dict[str, Any]:
    print("\n--- [3C] 3. Sentence-Chunked Streaming TTS Overlapped with SLM Generation ---")
    # Launch llama-server with Gemma 4 E2B
    server_cmd = [VULKAN_SERVER, "-m", E2B_PATH, "-ngl", "99", "--port", "8080", "-c", "2048", "--slots", "-fa", "on"]
    proc = subprocess.Popen(server_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    import urllib.request
    def wait_for_server():
        st = time.time()
        while time.time() - st < 40.0:
            try:
                with urllib.request.urlopen("http://127.0.0.1:8080/health", timeout=1.0) as resp:
                    if resp.status == 200:
                        return True
            except Exception:
                time.sleep(0.3)
        return False

    if not wait_for_server():
        print("  Error: Server failed to start for streaming TTS test.")
        if proc.poll() is None:
            proc.kill()
        return {}

    with open(PIPER_JSON, "r", encoding="utf-8") as f:
        tts_cfg = json.load(f)
    phoneme_map = tts_cfg.get("phoneme_id_map", {})
    scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)

    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 4
    tts_sess = ort.InferenceSession(PIPER_ONNX, opts, providers=["CPUExecutionProvider"])

    prompt = "Explain in two complete sentences how active noise cancellation and sidetone work together:"

    # 1. Non-streamed baseline: wait for complete SLM response, then synthesize full text
    t0 = time.perf_counter()
    url = "http://127.0.0.1:8080/completion"
    payload = json.dumps({"prompt": prompt, "n_predict": 64, "stream": False}).encode("utf-8")
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60.0) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    t1 = time.perf_counter()
    slm_full_text = data.get("content", "Active noise cancellation uses phase inversion. Sidetone mixes voice.")
    slm_generation_time_ms = (t1 - t0) * 1000.0

    p_ids_full = text_to_phonemes(slm_full_text, phoneme_map)
    inp_f = np.array([p_ids_full], dtype=np.int64)
    lens_f = np.array([len(p_ids_full)], dtype=np.int64)
    t2 = time.perf_counter()
    _ = tts_sess.run(None, {"input": inp_f, "input_lengths": lens_f, "scales": scales})
    t3 = time.perf_counter()
    non_streamed_tts_ms = (t3 - t2) * 1000.0
    non_streamed_ttfa_ms = slm_generation_time_ms + non_streamed_tts_ms

    # 2. Streamed overlapped: stream tokens, as soon as first sentence boundary (. / !) appears, synthesize clause
    t4 = time.perf_counter()
    payload_stream = json.dumps({"prompt": prompt, "n_predict": 64, "stream": True}).encode("utf-8")
    req_stream = urllib.request.Request(url, data=payload_stream, headers={"Content-Type": "application/json"})

    first_sentence = ""
    sentence_ready_time = None
    first_audio_time = None

    with urllib.request.urlopen(req_stream, timeout=60.0) as resp:
        buffer = ""
        for line in resp:
            line_str = line.decode("utf-8").strip()
            if line_str.startswith("data: "):
                chunk_json = line_str[6:]
                if chunk_json == "[DONE]":
                    break
                try:
                    chunk = json.loads(chunk_json)
                    token = chunk.get("content", "")
                    buffer += token
                    if any(punct in token for punct in [".", "!", "?"]) and not first_sentence:
                        first_sentence = buffer.strip()
                        sentence_ready_time = time.perf_counter()
                        # Synthesize first sentence immediately!
                        p_first = text_to_phonemes(first_sentence, phoneme_map)
                        inp_1 = np.array([p_first], dtype=np.int64)
                        lens_1 = np.array([len(p_first)], dtype=np.int64)
                        _ = tts_sess.run(None, {"input": inp_1, "input_lengths": lens_1, "scales": scales})
                        first_audio_time = time.perf_counter()
                except Exception:
                    pass

    t5 = time.perf_counter()

    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()

    streamed_ttfa_ms = (first_audio_time - t4) * 1000.0 if first_audio_time else non_streamed_ttfa_ms
    ttfa_reduction_ms = non_streamed_ttfa_ms - streamed_ttfa_ms
    ttfa_speedup_x = non_streamed_ttfa_ms / max(streamed_ttfa_ms, 0.1)

    res = {
        "non_streamed_slm_generation_ms": round(slm_generation_time_ms, 1),
        "non_streamed_tts_synthesis_ms": round(non_streamed_tts_ms, 1),
        "non_streamed_ttfa_ms": round(non_streamed_ttfa_ms, 1),
        "streamed_first_clause_ttfa_ms": round(streamed_ttfa_ms, 1),
        "ttfa_reduction_ms": round(ttfa_reduction_ms, 1),
        "ttfa_speedup_x": round(ttfa_speedup_x, 2)
    }

    print(f"  Non-Streamed TTFA : {res['non_streamed_ttfa_ms']} ms (Waited for full {res['non_streamed_slm_generation_ms']}ms SLM generation)")
    print(f"  Streamed Overlapped TTFA: {res['streamed_first_clause_ttfa_ms']} ms (Audio begins playing during generation)")
    print(f"  TTFA Improvement  : {res['ttfa_reduction_ms']} ms faster ({res['ttfa_speedup_x']}x improvement)")

    return res


def main():
    acquire_lock("Scenario_3C_ONNX_Audio")
    try:
        graph_res = benchmark_onnx_graph_serialization_and_arena()
        piper_tiered_res = benchmark_piper_tiered_synthesis_and_caching()
        streaming_tts_res = benchmark_streaming_overlapped_tts()

        output = {
            "scenario": "3C_STT_TTS_ONNX_Startup_Preloading_Streaming",
            "graph_optimization_and_arena": graph_res,
            "piper_tiered_synthesis": piper_tiered_res,
            "streaming_overlapped_tts": streaming_tts_res
        }

        out_path = r"C:\develop\mutop\logs\scenario_3c_summary.json"
        os.makedirs(os.path.dirname(out_path), exist_ok=True)
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump(output, f, indent=2)

        print("\n================================================================================")
        print("Scenario 3C Complete! Summary saved to scenario_3c_summary.json")
        print("================================================================================")

    finally:
        release_lock()


if __name__ == "__main__":
    main()
