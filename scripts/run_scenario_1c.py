#!/usr/bin/env python3
"""
run_scenario_1c.py - Scenario 1C: GPU SLM Gemma 4 E2B & E4B Runner on Intel Arc 140V GPU.

Coordinates exclusive execution via C:\develop\mutop\scripts\bench.lock.
Collects telemetry via mutop-ai -s 200 --mark-file ... --llama-port 8080 --proc llama-server --threads --summary.
Evaluates:
  - Gemma-4-E2B (Q4_K_M) on Arc 140V (Vulkan server PP512 + TG128, llama-bench Vulkan vs SYCL)
  - Gemma-4-E4B (Q4_K_M) on Arc 140V (Vulkan server PP512 + TG128, llama-bench Vulkan vs SYCL)
"""

import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from typing import Any, Dict, Optional


LOCK_FILE = r"C:\develop\mutop\scripts\bench.lock"
MARK_FILE = r"C:\develop\mutop\scripts\mutop_markers.txt"
MUTOP_EXE = r"C:\develop\mutop\mutop-ai.exe"
LOG_DIR = r"C:\develop\mutop\logs"
MODELS_DIR = r"C:\Users\peter\models\slm"

VULKAN_SERVER = r"C:\tools\llama.cpp-vulkan\llama-server.exe"
VULKAN_BENCH = r"C:\tools\llama.cpp-vulkan\llama-bench.exe"
SYCL_BENCH = r"C:\tools\llama.cpp-sycl\llama-bench.exe"


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


def query_completion(prompt: str, n_predict: int) -> Dict[str, Any]:
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
    total_time = t1 - t0
    timings = data.get("timings", {})
    return {
        "total_time_sec": total_time,
        "prompt_n": timings.get("prompt_n", 0),
        "predicted_n": timings.get("predicted_n", n_predict),
        "prompt_tok_s": timings.get("prompt_per_second", 0.0),
        "gen_tok_s": timings.get("predicted_per_second", 0.0),
    }


def parse_summary_report(log_path: str) -> Dict[str, Any]:
    stats: Dict[str, Any] = {}
    if not os.path.exists(log_path):
        return stats
    with open(log_path, "r", encoding="utf-8", errors="replace") as f:
        content = f.read()

    # Power
    m_pwr = re.search(r"Board Power \(W\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_pwr:
        stats["power_min_w"] = float(m_pwr.group(1))
        stats["power_median_w"] = float(m_pwr.group(2))
        stats["power_max_w"] = float(m_pwr.group(4))

    # Fan Speed
    m_fan = re.search(r"Fan Speed \(RPM\)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)", content)
    if m_fan:
        stats["fan_median_rpm"] = int(m_fan.group(2))
        stats["fan_max_rpm"] = int(m_fan.group(4))

    # CPU Total Load
    m_cpu = re.search(r"CPU Total Load \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_cpu:
        stats["cpu_total_median_pct"] = float(m_cpu.group(2))

    # GPU Active Duty
    m_gpu = re.search(r"GPU Active Duty \(%\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_gpu:
        stats["gpu_median_pct"] = float(m_gpu.group(2))
        stats["gpu_max_pct"] = float(m_gpu.group(4))

    # Thermals
    m_temp = re.search(r"cpu \(C\)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)\s+([\d\.]+)", content)
    if m_temp:
        stats["cpu_temp_median_c"] = float(m_temp.group(2))
        stats["cpu_temp_max_c"] = float(m_temp.group(4))

    # Active NPU/GPU share
    m_act = re.search(r"Active Intel AI Boost NPU & Arc 140V GPU Share:\s+([\d\.]+)%", content)
    if m_act:
        stats["npu_gpu_active_share_pct"] = float(m_act.group(1))

    return stats


def run_llama_bench(bin_path: str, model_path: str, p: int = 512, n: int = 128) -> str:
    if not os.path.exists(bin_path):
        return f"Binary not found: {bin_path}"
    cmd = [bin_path, "-m", model_path, "-ngl", "99", "-p", str(p), "-n", str(n), "-r", "2"]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=150)
        return proc.stdout.strip()
    except Exception as e:
        return f"Error: {e}"


def run_cell_1c(model_key: str, model_filename: str) -> Dict[str, Any]:
    cell_id = f"1C_{model_key}"
    model_path = os.path.join(MODELS_DIR, model_filename)
    log_file = os.path.join(LOG_DIR, f"scenario_1c_{model_key.lower()}.log")

    print(f"\n================================================================================")
    print(f">>> Scenario 1C: Benchmarking {model_key} on Intel Arc 140V GPU")
    print(f"================================================================================")

    if not os.path.exists(model_path):
        print(f"Error: Model not found at {model_path}")
        return {}

    acquire_lock(cell_id)

    # Reset mark file
    with open(MARK_FILE, "w", encoding="utf-8") as f:
        f.write("")

    # Start mutop-ai collector
    mutop_cmd = [
        MUTOP_EXE,
        "-s", "200",
        "--mark-file", MARK_FILE,
        "--proc", "llama-server",
        "--threads",
        "--summary"
    ]

    cell_results: Dict[str, Any] = {"model": model_key}

    with open(log_file, "w", encoding="utf-8") as out_f:
        mutop_proc = subprocess.Popen(mutop_cmd, stdout=out_f, stderr=subprocess.STDOUT)

    time.sleep(0.5)

    server_proc = None
    try:
        # Launch llama-server with Vulkan on Arc 140V
        server_cmd = [VULKAN_SERVER, "-m", model_path, "-ngl", "99", "--port", "8080", "--slots", "-c", "2048"]
        server_proc = subprocess.Popen(server_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        print("    Waiting for llama-server (Vulkan) to initialize on port 8080...")
        if not wait_for_server(timeout_sec=45.0):
            print("    Error: llama-server failed to initialize.")
            return {}

        prompt_512 = ("Overview of hardware telemetry and AI execution acceleration on Lunar Lake: " * 35)[:2500]

        # Phase 1: Prefill PP512
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"1C_{model_key}_pp512")
        time.sleep(0.5)
        print("    Running PP512 prefill query...")
        pp_res = query_completion(prompt_512, n_predict=1)
        print(f"    PP512 Throughput: {pp_res['prompt_tok_s']:.2f} tok/s (evaluated {pp_res['prompt_n']} tokens)")

        # Phase 2: Generation TG128
        with open(MARK_FILE, "w", encoding="utf-8") as f:
            f.write(f"1C_{model_key}_tg128")
        time.sleep(0.5)
        print("    Running TG128 decode query...")
        tg_res = query_completion("Describe the architecture of Intel Arc 140V: ", n_predict=128)
        print(f"    TG128 Throughput: {tg_res['gen_tok_s']:.2f} tok/s (predicted {tg_res['predicted_n']} tokens)")

        cell_results["server_vulkan"] = {
            "pp512_tok_s": round(pp_res["prompt_tok_s"], 2),
            "tg128_tok_s": round(tg_res["gen_tok_s"], 2),
            "pp_tokens": pp_res["prompt_n"],
            "tg_tokens": tg_res["predicted_n"],
        }
    finally:
        if server_proc:
            server_proc.terminate()
            try:
                server_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server_proc.kill()

        # Signal mutop-ai termination
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

    # Check if bench results already exist in prior summary
    prior_summary_path = os.path.join(LOG_DIR, "scenario_1c_summary.json")
    prior_bench_vk = None
    prior_bench_sycl = None
    if os.path.exists(prior_summary_path):
        try:
            with open(prior_summary_path, "r", encoding="utf-8") as f:
                prior = json.load(f)
            prior_bench_vk = prior.get(f"Gemma-4-{model_key}", {}).get("bench_vulkan")
            prior_bench_sycl = prior.get(f"Gemma-4-{model_key}", {}).get("bench_sycl")
        except Exception:
            pass

    if prior_bench_vk:
        cell_results["bench_vulkan"] = prior_bench_vk
    if prior_bench_sycl:
        cell_results["bench_sycl"] = prior_bench_sycl

    if not prior_bench_vk:
        acquire_lock(f"{cell_id}_bench")
        try:
            print("\n    Running llama-bench Vulkan...")
            bench_vk = run_llama_bench(VULKAN_BENCH, model_path, p=512, n=128)
            cell_results["bench_vulkan"] = bench_vk
            print(bench_vk)

            print("\n    Running llama-bench SYCL...")
            bench_sycl = run_llama_bench(SYCL_BENCH, model_path, p=512, n=128)
            cell_results["bench_sycl"] = bench_sycl
            print(bench_sycl)
        finally:
            release_lock()

    return cell_results


def main():
    os.makedirs(LOG_DIR, exist_ok=True)
    all_results = {}

    # 1. Gemma 4 E2B
    res_e2b = run_cell_1c("E2B", "gemma-4-E2B-it-Q4_K_M.gguf")
    all_results["Gemma-4-E2B"] = res_e2b

    # 2. Gemma 4 E4B
    res_e4b = run_cell_1c("E4B", "gemma-4-E4B-it-Q4_K_M.gguf")
    all_results["Gemma-4-E4B"] = res_e4b

    summary_path = os.path.join(LOG_DIR, "scenario_1c_summary.json")
    with open(summary_path, "w", encoding="utf-8") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nScenario 1C completed. Summary written to {summary_path}")


if __name__ == "__main__":
    main()
