#!/usr/bin/env python3
"""
benchmark_3d_workflows.py - Scenario 3D: End-to-End Target Voice Workflows: Cold vs. Warm Optimized Comparison.

Monitored by mutop-ai with phase markers:
  - 3D_wf1_cold_unoptimized
  - 3D_wf1_warm_s3_turn
  - 3D_wf1_warm_fast_path
  - 3D_wf2_cold_unoptimized
  - 3D_wf2_warm_streamed
"""

import json
import os
import subprocess
import sys
import time
import urllib.request
from typing import Any, Dict
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

E2B_PATH = os.path.join(MODELS_ROOT, "slm", "gemma-4-E2B-it-Q4_K_M.gguf")
FG_DIR = os.path.join(MODELS_ROOT, "llm", "FunctionGemma-270M-INT8")
EMBED_XML = os.path.join(MODELS_ROOT, "embedding", "EmbeddingGemma-300M", "openvino_model.xml")
CACHE_DIR = os.path.join(MODELS_ROOT, ".ov_cache")
MOONSHINE_TINY_ONNX = os.path.join(MODELS_ROOT, "stt", "moonshine", "tiny", "onnx", "encoder_model.onnx")
PIPER_ONNX = os.path.join(MODELS_ROOT, "tts", "piper", "en_GB-alan-medium.onnx")
PIPER_JSON = os.path.join(MODELS_ROOT, "tts", "piper", "en_GB-alan-medium.onnx.json")


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


def set_marker(label: str) -> None:
    with open(MARK_FILE, "w", encoding="utf-8") as f:
        f.write(label)
    time.sleep(0.3)


def text_to_phonemes(text: str, phoneme_map: dict) -> list:
    res = [0]
    for char in text.lower():
        val = phoneme_map.get(char, [1])[0]
        res.append(val)
        res.append(0)
    return res


def main():
    os.makedirs(LOG_DIR, exist_ok=True)
    scenario_id = "Scenario_3D_End_to_End_Workflows"
    log_file = os.path.join(LOG_DIR, "scenario_3d_workflows.log")

    print("================================================================================")
    print(">>> Scenario 3D: End-to-End Target Voice Workflows (Cold vs. Warm Optimized)")
    print("================================================================================")

    acquire_lock(scenario_id)

    # 1. Reset marker file
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

    wf1_results = {}
    wf2_results = {}

    try:
        # Load audio test sample (10s mono audio)
        audio_dur = 10.0
        t_arr = np.linspace(0, audio_dur, int(audio_dur * 16000), dtype=np.float32)
        test_audio = (0.5 * np.sin(2 * np.pi * 440 * t_arr)).reshape(1, -1)

        with open(PIPER_JSON, "r", encoding="utf-8") as f:
            tts_cfg = json.load(f)
        phoneme_map = tts_cfg.get("phoneme_id_map", {})
        tts_scales = np.array([0.667, 1.0, 0.8], dtype=np.float32)

        # ----------------------------------------------------------------------
        # WORKFLOW 1: Local Edge Control / Reflex Turn
        # Audio -> STT -> FunctionGemma NPU + EmbeddingGemma NPU -> Piper TTS
        # ----------------------------------------------------------------------
        print("\n>>> [3D] Workflow 1: Local Edge Control Turn...")

        # 1A. Cold Unoptimized
        set_marker("3D_wf1_cold_unoptimized")
        print("  Running Workflow 1 (a: Cold Unoptimized)...")
        t_cold_start = time.perf_counter()

        # Cold STT
        opts_cold = ort.SessionOptions()
        opts_cold.intra_op_num_threads = 4
        sess_stt_cold = ort.InferenceSession(MOONSHINE_TINY_ONNX, opts_cold, providers=["CPUExecutionProvider"])
        _ = sess_stt_cold.run(None, {sess_stt_cold.get_inputs()[0].name: test_audio})
        t_cold_stt = time.perf_counter()

        # Cold NPU Intent (Using a temporary un-cached dir to force cold compile)
        tmp_cold_cache = os.path.join(MODELS_ROOT, ".ov_cache_cold_tmp")
        os.makedirs(tmp_cold_cache, exist_ok=True)
        npu_pipe_cold = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=tmp_cold_cache)
        res_fg = npu_pipe_cold.generate("turn on active noise cancellation", max_new_tokens=24)
        t_cold_npu = time.perf_counter()

        # Cold TTS
        sess_tts_cold = ort.InferenceSession(PIPER_ONNX, opts_cold, providers=["CPUExecutionProvider"])
        p_ids = text_to_phonemes("Noise cancelling on.", phoneme_map)
        _ = sess_tts_cold.run(None, {"input": np.array([p_ids], dtype=np.int64), "input_lengths": np.array([len(p_ids)], dtype=np.int64), "scales": tts_scales})
        t_cold_end = time.perf_counter()

        wf1_cold_total_sec = t_cold_end - t_cold_start
        wf1_results["cold_unoptimized_sec"] = round(wf1_cold_total_sec, 3)
        print(f"    Workflow 1 (Cold Unoptimized) Total: {wf1_results['cold_unoptimized_sec']} s (NPU cold compile: {t_cold_npu - t_cold_stt:.2f}s)")

        del sess_stt_cold, npu_pipe_cold, sess_tts_cold
        import shutil
        shutil.rmtree(tmp_cold_cache, ignore_errors=True)

        # 1B. Warm Optimized S3 Turn
        set_marker("3D_wf1_warm_s3_turn")
        print("  Running Workflow 1 (b: Warm S3 FunctionGemma Turn)...")
        # Pre-warmed sessions
        sess_stt_warm = ort.InferenceSession(MOONSHINE_TINY_ONNX, opts_cold, providers=["CPUExecutionProvider"])
        _ = sess_stt_warm.run(None, {sess_stt_warm.get_inputs()[0].name: test_audio[:, :16000]})  # 1-frame warmup

        npu_pipe_warm = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=CACHE_DIR)
        _ = npu_pipe_warm.generate("warmup", max_new_tokens=16)

        sess_tts_warm = ort.InferenceSession(PIPER_ONNX, opts_cold, providers=["CPUExecutionProvider"])

        t_warm_s3_start = time.perf_counter()
        _ = sess_stt_warm.run(None, {sess_stt_warm.get_inputs()[0].name: test_audio})
        t_w1 = time.perf_counter()
        _ = npu_pipe_warm.generate("turn on noise cancelling", max_new_tokens=24)
        t_w2 = time.perf_counter()
        _ = sess_tts_warm.run(None, {"input": np.array([p_ids], dtype=np.int64), "input_lengths": np.array([len(p_ids)], dtype=np.int64), "scales": tts_scales})
        t_w3 = time.perf_counter()

        wf1_warm_s3_total_sec = t_w3 - t_warm_s3_start
        wf1_results["warm_s3_turn_sec"] = round(wf1_warm_s3_total_sec, 3)
        wf1_results["warm_s3_stt_ms"] = round((t_w1 - t_warm_s3_start) * 1000.0, 1)
        wf1_results["warm_s3_npu_ms"] = round((t_w2 - t_w1) * 1000.0, 1)
        wf1_results["warm_s3_tts_ms"] = round((t_w3 - t_w2) * 1000.0, 1)
        print(f"    Workflow 1 (Warm S3 Turn) Total: {wf1_results['warm_s3_turn_sec']} s (STT: {wf1_results['warm_s3_stt_ms']}ms, NPU: {wf1_results['warm_s3_npu_ms']}ms, TTS: {wf1_results['warm_s3_tts_ms']}ms)")

        # 1C. Warm Optimized Fast-Path Turn (Embedding lookup + Pre-synthesized RAM reflex cache)
        set_marker("3D_wf1_warm_fast_path")
        print("  Running Workflow 1 (c: Warm Fast-Path Turn)...")
        ov_core = ov.Core()
        ov_core.set_property("NPU", {"CACHE_DIR": CACHE_DIR})
        embed_model = ov_core.read_model(EMBED_XML)
        embed_model.reshape({"input_ids": [1, 128], "attention_mask": [1, 128], "position_ids": [1, 128]})
        compiled_embed = ov_core.compile_model(embed_model, "NPU")
        embed_req = compiled_embed.create_infer_request()
        dummy_128 = {
            "input_ids": np.ones((1, 128), dtype=np.int64),
            "attention_mask": np.ones((1, 128), dtype=np.int64),
            "position_ids": np.arange(128, dtype=np.int64).reshape((1, 128))
        }
        embed_req.infer(dummy_128)

        ram_reflex_cache = {"noise_cancelling_on": b"\x00" * 48000}

        t_fp_start = time.perf_counter()
        _ = sess_stt_warm.run(None, {sess_stt_warm.get_inputs()[0].name: test_audio})
        t_fp1 = time.perf_counter()
        embed_req.infer(dummy_128)
        t_fp2 = time.perf_counter()
        _ = ram_reflex_cache.get("noise_cancelling_on")
        t_fp3 = time.perf_counter()

        wf1_fast_path_total_sec = t_fp3 - t_fp_start
        wf1_results["warm_fast_path_sec"] = round(wf1_fast_path_total_sec, 3)
        wf1_results["fast_path_stt_ms"] = round((t_fp1 - t_fp_start) * 1000.0, 1)
        wf1_results["fast_path_embed_ms"] = round((t_fp2 - t_fp1) * 1000.0, 1)
        wf1_results["fast_path_audio_cache_ms"] = round((t_fp3 - t_fp2) * 1000.0, 3)
        print(f"    Workflow 1 (Fast-Path Turn) Total: {wf1_results['warm_fast_path_sec']} s (STT: {wf1_results['fast_path_stt_ms']}ms, Embed: {wf1_results['fast_path_embed_ms']}ms, Cache: {wf1_results['fast_path_audio_cache_ms']}ms)")

        del sess_stt_warm, npu_pipe_warm, sess_tts_warm, compiled_embed

        # ----------------------------------------------------------------------
        # WORKFLOW 2: Generative Assistance Turn (Local SLM Reasoning)
        # Audio -> STT -> Gemma 4 E2B GPU -> Streamed Piper TTS
        # ----------------------------------------------------------------------
        print("\n>>> [3D] Workflow 2: Generative Assistance SLM Turn...")

        system_prompt = (
            "<system>You are Device Commander. Answer the user wearing smart headphones concisely in 2 sentences.</system>\n"
            "<user>Why does voice feedback sidetone make calls feel more natural?</user>\n<assistant>"
        )

        # 2A. Cold Unoptimized
        set_marker("3D_wf2_cold_unoptimized")
        print("  Running Workflow 2 (a: Cold Unoptimized)...")
        # Cold start llama-server with default flags and no pre-warmed slot
        cmd_cold = [VULKAN_SERVER, "-m", E2B_PATH, "-ngl", "99", "--port", "8080", "-c", "2048"]
        t_wf2_cold_start = time.perf_counter()
        proc_gpu_cold = subprocess.Popen(cmd_cold, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        wait_start = time.perf_counter()
        while time.perf_counter() - wait_start < 45.0:
            try:
                with urllib.request.urlopen("http://127.0.0.1:8080/health", timeout=1.0) as resp:
                    if resp.status == 200:
                        break
            except Exception:
                time.sleep(0.4)

        # Query complete non-streamed response
        url = "http://127.0.0.1:8080/completion"
        payload = json.dumps({"prompt": system_prompt, "n_predict": 64, "stream": False}).encode("utf-8")
        req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=60.0) as resp:
            data_resp = json.loads(resp.read().decode("utf-8"))
        t_gpu_done = time.perf_counter()

        # Non-streamed TTS after generation completes
        sess_tts_cold2 = ort.InferenceSession(PIPER_ONNX, opts_cold, providers=["CPUExecutionProvider"])
        full_text = data_resp.get("content", "Sidetone lets you hear your voice naturally so you do not shout.")
        p_ids_w2 = text_to_phonemes(full_text, phoneme_map)
        _ = sess_tts_cold2.run(None, {"input": np.array([p_ids_w2], dtype=np.int64), "input_lengths": np.array([len(p_ids_w2)], dtype=np.int64), "scales": tts_scales})
        t_wf2_cold_end = time.perf_counter()

        proc_gpu_cold.terminate()
        try:
            proc_gpu_cold.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc_gpu_cold.kill()

        wf2_cold_total_sec = t_wf2_cold_end - t_wf2_cold_start
        wf2_results["cold_unoptimized_total_sec"] = round(wf2_cold_total_sec, 3)
        wf2_results["cold_unoptimized_ttfa_sec"] = round(t_gpu_done - t_wf2_cold_start, 3)
        print(f"    Workflow 2 (Cold) Total: {wf2_results['cold_unoptimized_total_sec']} s | TTFA: {wf2_results['cold_unoptimized_ttfa_sec']} s")

        del sess_tts_cold2

        # 2B. Warm Preloaded & Streamed
        set_marker("3D_wf2_warm_streamed")
        print("  Running Workflow 2 (b: Warm Preloaded & Streamed)...")
        # Launch server with FlashAttn + Q8_0 KV + slot caching
        cmd_warm = [
            VULKAN_SERVER,
            "-m", E2B_PATH,
            "-ngl", "99",
            "--port", "8080",
            "-c", "2048",
            "--slots",
            "-fa", "on",
            "-ctk", "q8_0",
            "-ctv", "q8_0",
            "--cache-reuse", "64",
            "-sps", "0.10"
        ]
        proc_gpu_warm = subprocess.Popen(cmd_warm, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        st = time.perf_counter()
        while time.perf_counter() - st < 45.0:
            try:
                with urllib.request.urlopen("http://127.0.0.1:8080/health", timeout=1.0) as resp:
                    if resp.status == 200:
                        break
            except Exception:
                time.sleep(0.4)

        # Pre-warm system prompt in slot
        _ = urllib.request.urlopen(urllib.request.Request(
            url,
            data=json.dumps({"prompt": system_prompt, "n_predict": 1, "stream": False}).encode("utf-8"),
            headers={"Content-Type": "application/json"}
        ), timeout=30.0)

        # Preloaded TTS session
        sess_tts_warm2 = ort.InferenceSession(PIPER_ONNX, opts_cold, providers=["CPUExecutionProvider"])

        t_wf2_warm_start = time.perf_counter()
        req_stream = urllib.request.Request(
            url,
            data=json.dumps({"prompt": system_prompt, "n_predict": 64, "stream": True}).encode("utf-8"),
            headers={"Content-Type": "application/json"}
        )

        wf2_first_sentence = ""
        wf2_first_audio_t = None

        with urllib.request.urlopen(req_stream, timeout=60.0) as resp:
            buf = ""
            for line in resp:
                l = line.decode("utf-8").strip()
                if l.startswith("data: "):
                    ck = l[6:]
                    if ck == "[DONE]":
                        break
                    try:
                        token_obj = json.loads(ck)
                        tok = token_obj.get("content", "")
                        buf += tok
                        if any(p in tok for p in [".", "!", "?"]) and not wf2_first_sentence:
                            wf2_first_sentence = buf.strip()
                            # Stream first sentence to TTS immediately!
                            p_tok = text_to_phonemes(wf2_first_sentence, phoneme_map)
                            _ = sess_tts_warm2.run(None, {"input": np.array([p_tok], dtype=np.int64), "input_lengths": np.array([len(p_tok)], dtype=np.int64), "scales": tts_scales})
                            wf2_first_audio_t = time.perf_counter()
                    except Exception:
                        pass

        t_wf2_warm_end = time.perf_counter()

        proc_gpu_warm.terminate()
        try:
            proc_gpu_warm.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc_gpu_warm.kill()

        wf2_ttfa_sec = (wf2_first_audio_t - t_wf2_warm_start) if wf2_first_audio_t else (t_wf2_warm_end - t_wf2_warm_start)
        wf2_total_sec = t_wf2_warm_end - t_wf2_warm_start

        wf2_results["warm_streamed_ttfa_sec"] = round(wf2_ttfa_sec, 3)
        wf2_results["warm_streamed_total_sec"] = round(wf2_total_sec, 3)
        wf2_results["ttfa_speedup_vs_cold_x"] = round(wf2_results["cold_unoptimized_ttfa_sec"] / max(wf2_ttfa_sec, 0.01), 2)
        print(f"    Workflow 2 (Warm Streamed) TTFA: {wf2_results['warm_streamed_ttfa_sec']} s | Total: {wf2_results['warm_streamed_total_sec']} s ({wf2_results['ttfa_speedup_vs_cold_x']}x faster TTFA)")

    finally:
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

    final_output = {
        "scenario": "3D_end_to_end_voice_workflows_cold_vs_warm",
        "workflow_1_edge_control": wf1_results,
        "workflow_2_generative_slm": wf2_results,
        "log_file": log_file
    }

    summary_json_path = os.path.join(LOG_DIR, "scenario_3d_summary.json")
    with open(summary_json_path, "w", encoding="utf-8") as f:
        json.dump(final_output, f, indent=2)

    print("\n================================================================================")
    print("Scenario 3D Complete! Summary saved to scenario_3d_summary.json")
    print("================================================================================")


if __name__ == "__main__":
    main()
