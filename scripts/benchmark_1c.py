#!/usr/bin/env python3
"""
benchmark_1c.py - Scenario 1C: GPU SLM Gemma 4 E2B & E4B Benchmark on Intel Arc 140V GPU.

Evaluates Gemma 4 E2B and E4B on Intel Arc 140V GPU (Xe2-LPG) using llama-server (Vulkan/SYCL)
and llama-bench, measuring:
  - Prompt Processing (PP512) throughput (tok/s) & Time-to-First-Token (TTFT)
  - Token Generation (TG128) throughput (tok/s)
  - Vulkan vs. SYCL backend comparison
  - Co-monitored by mutop-ai for GPU busy %, VRAM, package power, thermals, and fan RPM.
"""

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.request
import urllib.error

try:
    from common_markers import MarkerWriter
except ImportError:
    from scripts.common_markers import MarkerWriter


def wait_for_server(url: str, timeout_sec: float = 30.0) -> bool:
    """Waits until llama-server responds on /health or /slots."""
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


def query_completion(base_url: str, prompt: str, n_predict: int) -> dict:
    """Sends a completion request and measures latency, tok/s, and TTFT."""
    url = f"{base_url}/completion"
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
    total_time = t1 - t0
    timings = data.get("timings", {})
    return {
        "total_time_sec": total_time,
        "tokens_predicted": timings.get("predicted_n", n_predict),
        "tokens_evaluated": timings.get("prompt_n", 0),
        "prompt_tok_s": timings.get("prompt_per_second", 0.0),
        "gen_tok_s": timings.get("predicted_per_second", 0.0),
        "raw_response": data.get("content", "")[:100]
    }


def run_llama_bench(bin_path: str, model_path: str, p: int = 512, n: int = 128, reps: int = 3) -> str:
    """Runs llama-bench.exe and returns stdout."""
    if not os.path.exists(bin_path):
        return f"Binary not found: {bin_path}"
    cmd = [bin_path, "-m", model_path, "-ngl", "99", "-p", str(p), "-n", str(n), "-r", str(reps)]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        return proc.stdout.strip()
    except Exception as e:
        return f"llama-bench execution error: {e}"


def run_scenario_1c(models_dir: str, mark_file: Optional[str] = None, log_file: Optional[str] = None):
    writer = MarkerWriter(mark_file) if mark_file else None
    results = {}

    vulkan_server_bin = r"C:\tools\llama.cpp-vulkan\llama-server.exe"
    vulkan_bench_bin = r"C:\tools\llama.cpp-vulkan\llama-bench.exe"
    sycl_bench_bin = r"C:\tools\llama.cpp-sycl\llama-bench.exe"

    models = [
        ("Gemma-4-E2B", os.path.join(models_dir, "slm", "gemma-4-E2B-it-Q4_K_M.gguf")),
        ("Gemma-4-E4B", os.path.join(models_dir, "slm", "gemma-4-E4B-it-Q4_K_M.gguf")),
    ]

    # Fixed synthetic prompts for PP512 and TG128
    prompt_512 = ("Technical report on hardware telemetry and Intel Arc 140V Xe2-LPG compute architecture: " * 35)[:2500]

    for model_name, model_path in models:
        if not os.path.exists(model_path):
            print(f"Skipping {model_name}: {model_path} does not exist.")
            continue

        print(f"\n================================================================================")
        print(f"  Benchmarking {model_name} on Intel Arc 140V GPU (Vulkan)")
        print(f"================================================================================")

        # 1. Start llama-server on port 8080
        server_cmd = [vulkan_server_bin, "-m", model_path, "-ngl", "99", "--port", "8080", "--slots", "-c", "2048"]
        server_proc = subprocess.Popen(server_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            print("Waiting for llama-server to initialize...")
            if not wait_for_server("http://127.0.0.1:8080", timeout_sec=40.0):
                print(f"Error: llama-server failed to respond on 8080 for {model_name}")
                continue

            # Phase A: Prompt Processing (PP512)
            prefix = "1c_gpu_" + model_name.lower().replace("-", "_")
            if writer:
                writer.set_marker(f"{prefix}_pp512")
            print(f"Running PP512 prefill benchmark on {model_name}...")
            time.sleep(0.5)
            pp_res = query_completion("http://127.0.0.1:8080", prompt_512, n_predict=1)
            print(f"  PP512: {pp_res['prompt_tok_s']:.2f} tok/s (evaluated {pp_res['tokens_evaluated']} tokens)")

            # Phase B: Token Generation (TG128)
            if writer:
                writer.set_marker(f"{prefix}_tg128")
            print(f"Running TG128 decode benchmark on {model_name}...")
            time.sleep(0.5)
            tg_res = query_completion("http://127.0.0.1:8080", "Provide a brief system summary: ", n_predict=128)
            print(f"  TG128: {tg_res['gen_tok_s']:.2f} tok/s (predicted {tg_res['tokens_predicted']} tokens)")

            results[f"{model_name}_Vulkan_Server"] = {
                "pp512_tok_s": pp_res["prompt_tok_s"],
                "tg128_tok_s": tg_res["gen_tok_s"],
                "pp_eval_tokens": pp_res["tokens_evaluated"],
                "tg_pred_tokens": tg_res["tokens_predicted"],
            }
        finally:
            server_proc.terminate()
            try:
                server_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server_proc.kill()
            time.sleep(1.0)

        # 2. Run standalone llama-bench (Vulkan vs SYCL)
        print(f"\nRunning llama-bench Vulkan on {model_name}...")
        vk_bench_out = run_llama_bench(vulkan_bench_bin, model_path, p=512, n=128, reps=2)
        print(vk_bench_out)

        print(f"\nRunning llama-bench SYCL on {model_name}...")
        sycl_bench_out = run_llama_bench(sycl_bench_bin, model_path, p=512, n=128, reps=2)
        print(sycl_bench_out)

        results[f"{model_name}_Vulkan_Bench"] = vk_bench_out
        results[f"{model_name}_SYCL_Bench"] = sycl_bench_out

    if writer:
        writer.set_marker("1C_done")

    # Save summary
    if log_file:
        os.makedirs(os.path.dirname(os.path.abspath(log_file)), exist_ok=True)
        with open(log_file, "w", encoding="utf-8") as f:
            f.write(json.dumps(results, indent=2))
        print(f"\nSaved Scenario 1C benchmark results to {log_file}")

    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--models-dir", default=r"C:\Users\peter\models")
    parser.add_argument("--mark-file", default=None)
    parser.add_argument("--log-file", default=r"C:\develop\mutop\logs\scenario_1c_gpu_slm.log")
    args = parser.parse_args()
    run_scenario_1c(args.models_dir, args.mark_file, args.log_file)
