#!/usr/bin/env python3
"""
benchmark_3b_gpu.py - Scenario 3B (Part 2): GPU SLM Startup, System-Prompt KV Prefix Caching & UMA Bandwidth Relief.

Evaluates on Intel Arc 140V Xe2-LPG GPU (llama-server.exe Vulkan):
  1. Startup & Load Modes (--mmap vs. --no-mmap vs. --mlock)
  2. System-Prompt + Tool-Schema KV Prefix Caching (--cache-reuse 64 / Slot 0 Warm Prefix)
  3. UMA Bandwidth Relief (-fa + -ctk q8_0 -ctv q8_0 vs default F16 KV)
     and concurrent NPU FunctionGemma contention delta.
"""

import json
import os
import subprocess
import sys
import time
import urllib.request
from typing import Any, Dict, List
import openvino_genai as ov_genai


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
VULKAN_SERVER = r"C:\tools\llama.cpp-vulkan\llama-server.exe"
MODELS_ROOT = r"C:\Users\peter\models"
E2B_PATH = os.path.join(MODELS_ROOT, "slm", "gemma-4-E2B-it-Q4_K_M.gguf")
E4B_PATH = os.path.join(MODELS_ROOT, "slm", "gemma-4-E4B-it-Q4_K_M.gguf")
FG_DIR = os.path.join(MODELS_ROOT, "llm", "FunctionGemma-270M-INT8")
CACHE_DIR = os.path.join(MODELS_ROOT, ".ov_cache")


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


def wait_for_server(url: str = "http://127.0.0.1:8080", timeout_sec: float = 45.0) -> bool:
    start = time.time()
    while time.time() - start < timeout_sec:
        try:
            req = urllib.request.Request(f"{url}/health")
            with urllib.request.urlopen(req, timeout=1.0) as resp:
                if resp.status == 200:
                    return True
        except Exception:
            time.sleep(0.3)
    return False


def query_completion(prompt: str, n_predict: int = 128) -> Dict[str, Any]:
    url = "http://127.0.0.1:8080/completion"
    payload = json.dumps({
        "prompt": prompt,
        "n_predict": n_predict,
        "temperature": 0.0,
        "stream": False,
        "cache_prompt": True
    }).encode("utf-8")
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=90.0) as resp:
        t1 = time.perf_counter()
        data = json.loads(resp.read().decode("utf-8"))
    timings = data.get("timings", {})
    return {
        "duration_sec": t1 - t0,
        "prompt_n": timings.get("prompt_n", 0),
        "prompt_ms": timings.get("prompt_ms", 0.0),
        "prompt_per_second": timings.get("prompt_per_second", 0.0),
        "predicted_n": timings.get("predicted_n", 0),
        "predicted_ms": timings.get("predicted_ms", 0.0),
        "predicted_per_second": timings.get("predicted_per_second", 0.0),
        "ttft_ms": (timings.get("prompt_ms", 0.0) or (t1 - t0) * 1000.0),
    }


def terminate_process(proc: subprocess.Popen) -> None:
    if proc and proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        time.sleep(0.5)


def run_load_mode_tests() -> List[Dict[str, Any]]:
    print("\n--- [3B-Part2] 1. Startup & Load Mode Evaluation (Gemma 4 E2B) ---")
    results = []
    modes = [
        ("mmap", ["-lm", "mmap"]),
        ("none (no-mmap)", ["-lm", "none"]),
        ("mmap+mlock", ["-lm", "mmap+mlock"]),
    ]

    for mode_name, flags in modes:
        print(f"  Testing Load Mode: {mode_name}...")
        cmd = [VULKAN_SERVER, "-m", E2B_PATH, "-ngl", "99", "--port", "8080", "-c", "1024"] + flags
        t_spawn0 = time.perf_counter()
        proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        ready = wait_for_server(timeout_sec=50.0)
        t_ready = time.perf_counter()
        server_ready_sec = t_ready - t_spawn0

        if not ready:
            print(f"    Failed to start with {mode_name}")
            terminate_process(proc)
            continue

        # Measure cold first-request TTFT
        query_res = query_completion("Warmup prompt for cold first-token latency verification.", n_predict=16)
        terminate_process(proc)

        stat = {
            "mode": mode_name,
            "server_ready_sec": round(server_ready_sec, 3),
            "first_request_ttft_ms": round(query_res["ttft_ms"], 1),
            "total_cold_startup_to_first_token_sec": round(server_ready_sec + (query_res["ttft_ms"] / 1000.0), 3)
        }
        results.append(stat)
        print(f"    Ready in {stat['server_ready_sec']}s | First-Request TTFT: {stat['first_request_ttft_ms']}ms | Total: {stat['total_cold_startup_to_first_token_sec']}s")

    return results


def run_kv_prefix_caching_test() -> Dict[str, Any]:
    print("\n--- [3B-Part2] 2. System-Prompt KV Prefix Caching Evaluation ---")
    cmd = [
        VULKAN_SERVER,
        "-m", E2B_PATH,
        "-ngl", "99",
        "--port", "8080",
        "-c", "2048",
        "--slots",
        "--cache-reuse", "64",
        "-sps", "0.10"
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not wait_for_server(timeout_sec=40.0):
        print("  Error: Server failed to start for KV prefix caching.")
        terminate_process(proc)
        return {}

    # Realistic 400-token system prompt defining Device Commander rules and tools
    system_prompt = (
        "<system>\n"
        "You are Device Commander, an audio-first eyes-free smart assistant for Jabra smart hardware. "
        "You control acoustic settings, hearing protection, ANC, HearThrough, equalizer profiles, and telephony.\n"
        "Supported Tools:\n"
        "- set_setting(name: str, value: str): Adjusts discrete parameters like ancMode (off, anc, hearThrough), sidetoneLevel (0..15), voicePrompts.\n"
        "- adjust_setting(name: str, direction: str, magnitude: str): Relative adjustments (volume up/down, sidetone warmer/clearer).\n"
        "- start_scene(name: str): Life scene automation (focus_work, windy_walk, commute, office_presentation).\n"
        "- report_symptom(symptom: str): Perceptual mapping (occlusion, muffled, hollow, harsh, noisy).\n"
        "Rules: Respond concisely within 3 items and 3 seconds. For high-impact actions like factory reset, require explicit confirmation.\n"
        "</system>\n\n"
    )

    user_turn_1 = system_prompt + "<user>Turn on active noise cancellation and set voice feedback sidetone to level four.</user>\n<assistant>"
    user_turn_2 = system_prompt + "<user>What is my current battery percentage and is the busylight active?</user>\n<assistant>"

    print("  Executing Turn 1 (Cold full prompt evaluation)...")
    res_turn_1 = query_completion(user_turn_1, n_predict=24)

    print("  Executing Turn 2 (Warm prompt sharing exact 400-token system prefix)...")
    res_turn_2 = query_completion(user_turn_2, n_predict=24)

    terminate_process(proc)

    result = {
        "turn_1_cold": {
            "prompt_tokens_evaluated": res_turn_1["prompt_n"],
            "prompt_ms": round(res_turn_1["prompt_ms"], 1),
            "prefill_tok_s": round(res_turn_1["prompt_per_second"], 1),
            "ttft_ms": round(res_turn_1["ttft_ms"], 1)
        },
        "turn_2_warm_prefix": {
            "prompt_tokens_evaluated": res_turn_2["prompt_n"],
            "prompt_ms": round(res_turn_2["prompt_ms"], 1),
            "prefill_tok_s": round(res_turn_2["prompt_per_second"], 1),
            "ttft_ms": round(res_turn_2["ttft_ms"], 1)
        },
        "prefix_prefill_speedup_x": round(res_turn_1["prompt_ms"] / max(res_turn_2["prompt_ms"], 0.1), 2),
        "prefill_latency_savings_ms": round(res_turn_1["prompt_ms"] - res_turn_2["prompt_ms"], 1)
    }

    print(f"  Turn 1 (Cold): {result['turn_1_cold']['prompt_tokens_evaluated']} tokens in {result['turn_1_cold']['prompt_ms']} ms ({result['turn_1_cold']['prefill_tok_s']} tok/s)")
    print(f"  Turn 2 (Warm): {result['turn_2_warm_prefix']['prompt_tokens_evaluated']} tokens in {result['turn_2_warm_prefix']['prompt_ms']} ms ({result['turn_2_warm_prefix']['prefill_tok_s']} tok/s)")
    print(f"  Prefix Caching Benefit: {result['prefix_prefill_speedup_x']}x faster prefill (-{result['prefill_latency_savings_ms']} ms)")

    return result


def run_bandwidth_and_contention_test() -> Dict[str, Any]:
    print("\n--- [3B-Part2] 3. UMA Bandwidth Relief & Concurrent NPU Contention ---")
    configs = [
        ("F16_KV_Default", []),
        ("Q8_0_KV_FlashAttn", ["-fa", "on", "-ctk", "q8_0", "-ctv", "q8_0"])
    ]

    # Initialize NPU FunctionGemma
    print("  Initializing NPU FunctionGemma-270M...")
    npu_pipe = ov_genai.LLMPipeline(FG_DIR, "NPU", CACHE_DIR=CACHE_DIR)
    _ = npu_pipe.generate("warmup", max_new_tokens=16)

    # 512-token prompt for prefill test
    prompt_512 = "Device Commander smart headset architecture test. " * 32

    results = {}

    for cfg_name, extra_flags in configs:
        print(f"\n  Evaluating Configuration: {cfg_name} on Gemma 4 E2B...")
        cmd = [VULKAN_SERVER, "-m", E2B_PATH, "-ngl", "99", "--port", "8080", "-c", "2048", "--slots"] + extra_flags
        proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not wait_for_server(timeout_sec=40.0):
            print("  Error starting server.")
            terminate_process(proc)
            continue

        # 1. Measure GPU solo PP512 and TG128
        res_gpu = query_completion(prompt_512, n_predict=128)

        # 2. Measure concurrent NPU latency while GPU is generating
        import threading
        npu_latencies = []
        stop_npu = threading.Event()

        def npu_worker():
            fg_prompts = [
                "turn on active noise cancelling",
                "set volume to seventy percent",
                "switch to hear through",
                "mute microphone",
                "check battery level"
            ]
            idx = 0
            while not stop_npu.is_set():
                t0 = time.perf_counter()
                _ = npu_pipe.generate(fg_prompts[idx % len(fg_prompts)], max_new_tokens=24)
                t1 = time.perf_counter()
                npu_latencies.append((t1 - t0) * 1000.0)
                idx += 1

        t_npu = threading.Thread(target=npu_worker)
        t_npu.start()

        # Run 2 GPU queries while NPU runs in parallel
        _ = query_completion(prompt_512, n_predict=128)
        _ = query_completion(prompt_512, n_predict=128)

        stop_npu.set()
        t_npu.join(timeout=5.0)

        terminate_process(proc)

        results[cfg_name] = {
            "gpu_pp512_tok_s": round(res_gpu["prompt_per_second"], 1),
            "gpu_tg128_tok_s": round(res_gpu["predicted_per_second"], 1),
            "concurrent_npu_turns_completed": len(npu_latencies),
            "concurrent_npu_avg_ms": round(float(sum(npu_latencies) / max(len(npu_latencies), 1)), 1),
            "concurrent_npu_min_ms": round(min(npu_latencies), 1) if npu_latencies else 0.0,
            "concurrent_npu_max_ms": round(max(npu_latencies), 1) if npu_latencies else 0.0,
        }

        print(f"    GPU PP512: {results[cfg_name]['gpu_pp512_tok_s']} tok/s | TG128: {results[cfg_name]['gpu_tg128_tok_s']} tok/s")
        print(f"    Concurrent NPU Intent Latency: avg {results[cfg_name]['concurrent_npu_avg_ms']} ms (min {results[cfg_name]['concurrent_npu_min_ms']} ms, max {results[cfg_name]['concurrent_npu_max_ms']} ms)")

    return results


def main():
    acquire_lock("Scenario_3B_Part2")
    try:
        load_mode_res = run_load_mode_tests()
        kv_prefix_res = run_kv_prefix_caching_test()
        bw_res = run_bandwidth_and_contention_test()

        output = {
            "scenario": "3B_Part2_GPU_SLM_Startup_Caching_Bandwidth",
            "load_modes": load_mode_res,
            "system_prompt_prefix_caching": kv_prefix_res,
            "uma_bandwidth_and_contention": bw_res
        }

        out_path = r"C:\develop\mutop\logs\scenario_3b_part2_summary.json"
        os.makedirs(os.path.dirname(out_path), exist_ok=True)
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump(output, f, indent=2)

        print("\n================================================================================")
        print("Scenario 3B (Part 2) Complete! Summary saved to scenario_3b_part2_summary.json")
        print("================================================================================")

    finally:
        release_lock()


if __name__ == "__main__":
    main()
