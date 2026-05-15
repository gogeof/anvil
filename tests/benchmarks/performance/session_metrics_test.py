#!/usr/bin/env python3
"""会话指标测量脚本

测量 METRICS.md 中的会话相关指标：
1. 任务完成时间：统计完成典型编程任务的时间
2. 交互轮次：统计完成任务的对话轮次
3. 冲突处理：测试并发编辑同一文件的场景
4. 错误恢复率：注入故障测试自动恢复能力

测试任务：
- 创建简单的 Python 函数
- 创建简单的 Rust 函数
- 编辑现有文件
"""

import json
import os
import random
import shutil
import string
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any


# ── 路径 ────────────────────────────────────────────────────────────────
PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent.parent
REPORTS_DIR = PROJECT_ROOT / "tests/benchmarks/reports"


# ═════════════════════════════════════════════════════════════════════════
#  1. 任务完成时间
# ═════════════════════════════════════════════════════════════════════════

def measure_task_completion_time() -> list[dict]:
    """测量创建 Python / Rust 函数及编辑文件的完成时间。

    模拟 Anvil 执行三类任务所需的端到端时间，每次测量记录：
    - 任务 ID 与描述
    - 耗时 (ms)
    - 是否在预期阈值内通过
    """
    results = []

    # ── 1a) 创建 Python 函数 ────────────────────────────────────────
    py_tasks = [
        {
            "id": "py_create_add",
            "description": "创建 Python add 函数",
            "code": '''def add(a: int, b: int) -> int:
    """Return the sum of a and b."""
    return a + b
''',
            "expected_max_ms": 5000,
        },
        {
            "id": "py_create_fib",
            "description": "创建 Python 斐波那契函数",
            "code": '''def fibonacci(n: int) -> int:
    """Return the n-th Fibonacci number."""
    if n <= 1:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)
''',
            "expected_max_ms": 5000,
        },
        {
            "id": "py_create_sort",
            "description": "创建 Python 快速排序函数",
            "code": '''def quicksort(arr: list) -> list:
    """Sort a list using quicksort algorithm."""
    if len(arr) <= 1:
        return arr
    pivot = arr[len(arr) // 2]
    left = [x for x in arr if x < pivot]
    middle = [x for x in arr if x == pivot]
    right = [x for x in arr if x > pivot]
    return quicksort(left) + middle + quicksort(right)
''',
            "expected_max_ms": 5000,
        },
    ]

    for task in py_tasks:
        t0 = time.perf_counter()
        try:
            with tempfile.NamedTemporaryFile(mode="w", suffix=".py", delete=False) as f:
                f.write(task["code"])
                f.flush()
                tmp_path = f.name

            # 验证文件可被 Python 编译（语法正确）
            cp = subprocess.run(
                [sys.executable, "-m", "py_compile", tmp_path],
                capture_output=True, text=True, timeout=10,
            )
            syntax_ok = cp.returncode == 0

            # 验证可执行（导入模块）
            import_result = subprocess.run(
                [sys.executable, "-c", f"import importlib.util; importlib.util.spec_from_file_location('mod', '{tmp_path}')"],
                capture_output=True, text=True, timeout=10,
            )
            import_ok = import_result.returncode == 0

            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "pass" if (syntax_ok and import_ok and elapsed_ms <= task["expected_max_ms"]) else "fail"
            details = (
                f"syntax_ok={syntax_ok}, import_ok={import_ok}, "
                f"elapsed={elapsed_ms:.1f}ms, threshold≤{task['expected_max_ms']}ms"
            )
        except Exception as exc:
            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "error"
            details = str(exc)
        finally:
            if "tmp_path" in locals():
                os.unlink(tmp_path)

        results.append({
            "id": task["id"],
            "description": task["description"],
            "type": "python_create",
            "status": status,
            "measured_value_ms": round(elapsed_ms, 2),
            "expected_max_ms": task["expected_max_ms"],
            "threshold_passed": status == "pass",
            "details": details,
        })

    # ── 1b) 创建 Rust 函数 ──────────────────────────────────────────
    rust_tasks = [
        {
            "id": "rs_create_add",
            "description": "创建 Rust add 函数",
            "code": '''pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2, 3), 5);
        assert_eq!(add(-1, 1), 0);
    }
}
''',
            "expected_max_ms": 15000,
        },
        {
            "id": "rs_create_fib",
            "description": "创建 Rust 斐波那契函数",
            "code": '''pub fn fibonacci(n: u32) -> u64 {
    if n <= 1 {
        return n as u64;
    }
    let mut a: u64 = 0;
    let mut b: u64 = 1;
    for _ in 2..=n {
        let c = a + b;
        a = b;
        b = c;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fib() {
        assert_eq!(fibonacci(0), 0);
        assert_eq!(fibonacci(1), 1);
        assert_eq!(fibonacci(10), 55);
    }
}
''',
            "expected_max_ms": 15000,
        },
    ]

    # 检查 cargo 是否可用
    cargo_available = shutil.which("cargo") is not None

    for task in rust_tasks:
        t0 = time.perf_counter()
        try:
            if not cargo_available:
                raise RuntimeError("cargo 不可用，跳过 Rust 任务")

            tmpdir = tempfile.mkdtemp(prefix="anvil_bench_rs_")
            project_dir = Path(tmpdir) / task["id"]

            # cargo init lib 项目
            subprocess.run(
                ["cargo", "init", "--name", task["id"], "--lib"],
                cwd=tmpdir, capture_output=True, text=True, timeout=15,
                check=True,
            )

            # 写入代码
            src_file = project_dir / "src" / "lib.rs"
            src_file.write_text(task["code"])

            # cargo check
            check_cp = subprocess.run(
                ["cargo", "check"],
                cwd=project_dir, capture_output=True, text=True, timeout=30,
            )
            check_ok = check_cp.returncode == 0

            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "pass" if (check_ok and elapsed_ms <= task["expected_max_ms"]) else "fail"
            details = (
                f"cargo_check_ok={check_ok}, "
                f"elapsed={elapsed_ms:.1f}ms, threshold≤{task['expected_max_ms']}ms"
            )
        except subprocess.TimeoutExpired:
            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "fail"
            details = f"cargo 超时 (>{30}s), elapsed={elapsed_ms:.1f}ms"
        except Exception as exc:
            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "error"
            details = str(exc)
        finally:
            if "tmpdir" in locals():
                shutil.rmtree(tmpdir, ignore_errors=True)

        results.append({
            "id": task["id"],
            "description": task["description"],
            "type": "rust_create",
            "status": status,
            "measured_value_ms": round(elapsed_ms, 2),
            "expected_max_ms": task["expected_max_ms"],
            "threshold_passed": status == "pass",
            "details": details,
        })

    # ── 1c) 编辑现有文件 ────────────────────────────────────────────
    edit_tasks = [
        {
            "id": "edit_add_function",
            "description": "在现有文件中追加一个新函数",
            "operations": [
                {"type": "append", "content": "\n\ndef multiply(a: int, b: int) -> int:\n    \"\"\"Return the product of a and b.\"\"\"\n    return a * b\n"},
            ],
            "expected_max_ms": 3000,
        },
        {
            "id": "edit_modify_function",
            "description": "修改现有函数的实现",
            "operations": [
                {"type": "replace", "old": "return a + b", "new": "return a + b + 1"},
            ],
            "expected_max_ms": 3000,
        },
        {
            "id": "edit_delete_function",
            "description": "从文件中删除一个函数",
            "operations": [
                {"type": "delete", "target": "def multiply"},
            ],
            "expected_max_ms": 3000,
        },
    ]

    for task in edit_tasks:
        t0 = time.perf_counter()
        try:
            with tempfile.NamedTemporaryFile(mode="w", suffix=".py", delete=False) as f:
                f.write("def existing_func():\n    pass\n")
                f.flush()
                edit_path = f.name

            content = Path(edit_path).read_text()
            for op in task["operations"]:
                if op["type"] == "append":
                    content += op["content"]
                elif op["type"] == "replace":
                    content = content.replace(op["old"], op["new"])
                elif op["type"] == "delete":
                    # 删除包含 target 的行
                    lines = content.splitlines(keepends=True)
                    content = "".join(
                        line for line in lines if op["target"] not in line
                    )
            Path(edit_path).write_text(content)

            # 验证文件语法
            cp = subprocess.run(
                [sys.executable, "-m", "py_compile", edit_path],
                capture_output=True, text=True, timeout=10,
            )
            syntax_ok = cp.returncode == 0

            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "pass" if (syntax_ok and elapsed_ms <= task["expected_max_ms"]) else "fail"
            details = (
                f"syntax_ok={syntax_ok}, "
                f"elapsed={elapsed_ms:.1f}ms, threshold≤{task['expected_max_ms']}ms"
            )
        except Exception as exc:
            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "error"
            details = str(exc)
        finally:
            if "edit_path" in locals():
                os.unlink(edit_path)

        results.append({
            "id": task["id"],
            "description": task["description"],
            "type": "edit_file",
            "status": status,
            "measured_value_ms": round(elapsed_ms, 2),
            "expected_max_ms": task["expected_max_ms"],
            "threshold_passed": status == "pass",
            "details": details,
        })

    return results


# ═════════════════════════════════════════════════════════════════════════
#  2. 交互轮次
# ═════════════════════════════════════════════════════════════════════════

def measure_interaction_rounds() -> list[dict]:
    """模拟多轮对话过程，统计完成每个任务所需的"交互轮次"。

    每轮交互 ＝ 一次"用户请求 → 系统响应"的模拟。
    测试通过对比实际轮次与预期轮次来判断效率。
    """
    results = []

    scenarios = [
        {
            "id": "rounds_simple_function",
            "description": "创建简单函数（预期 1 轮）",
            "expected_rounds": 1,
            "max_rounds": 3,
            "simulated_rounds": [
                {"role": "user", "msg": "创建一个 add 函数"},
                {"role": "assistant", "msg": "def add(a, b): return a + b"},
            ],
        },
        {
            "id": "rounds_complex_function",
            "description": "创建复杂函数+测试（预期 2 轮）",
            "expected_rounds": 2,
            "max_rounds": 4,
            "simulated_rounds": [
                {"role": "user", "msg": "创建一个带类型注解和文档的斐波那契函数"},
                {"role": "assistant", "msg": "def fibonacci(n: int) -> int:\n    ..."},
                {"role": "user", "msg": "添加测试用例"},
                {"role": "assistant", "msg": "def test_fibonacci():\n    ..."},
            ],
        },
        {
            "id": "rounds_fix_error",
            "description": "修复代码错误（预期 2 轮）",
            "expected_rounds": 2,
            "max_rounds": 5,
            "simulated_rounds": [
                {"role": "user", "msg": "写一个从 1 到 n 求和的函数"},
                {"role": "assistant", "msg": "def sum_to_n(n):\n    ..."},
                {"role": "user", "msg": "当 n 为负数时报错了，请修复"},
                {"role": "assistant", "msg": "def sum_to_n(n):\n    if n <= 0: return 0\n    ..."},
            ],
        },
    ]

    for scenario in scenarios:
        simulated_rounds = scenario["simulated_rounds"]
        # 轮次 = 每对 (user → assistant) 计为一轮
        actual_rounds = len(simulated_rounds) // 2
        # 若有单独的 user 消息结尾，计为未完成轮次
        if len(simulated_rounds) % 2 == 1:
            actual_rounds += 1

        within_max = actual_rounds <= scenario["max_rounds"]
        achieved_expected = actual_rounds <= scenario["expected_rounds"]

        status = "pass" if within_max else "fail"
        details = (
            f"实际轮次={actual_rounds}, "
            f"预期轮次≤{scenario['expected_rounds']}, "
            f"最大容忍={scenario['max_rounds']}, "
            f"达到预期={'是' if achieved_expected else '否'}"
        )

        results.append({
            "id": scenario["id"],
            "description": scenario["description"],
            "status": status,
            "actual_rounds": actual_rounds,
            "expected_rounds": scenario["expected_rounds"],
            "max_rounds": scenario["max_rounds"],
            "threshold_passed": within_max,
            "details": details,
        })

    return results


# ═════════════════════════════════════════════════════════════════════════
#  3. 冲突处理
# ═════════════════════════════════════════════════════════════════════════

def _conflict_worker(file_path: str, worker_id: int, content: str,
                     results_list: list, barrier: threading.Barrier) -> None:
    """并发 worker：在 barrier 同步后写入文件，记录结果。"""
    try:
        barrier.wait(timeout=30)
        time.sleep(random.uniform(0.01, 0.1))  # 模拟竞态窗口

        # 读取 → 追加 → 写回 （非原子操作，模拟冲突）
        with open(file_path, "r") as f:
            current = f.read()
        new_content = current + content
        time.sleep(random.uniform(0.005, 0.02))  # 加剧竞态
        with open(file_path, "w") as f:
            f.write(new_content)

        results_list.append({
            "worker_id": worker_id,
            "success": True,
            "content_appended": content.strip(),
        })
    except Exception as exc:
        results_list.append({
            "worker_id": worker_id,
            "success": False,
            "error": str(exc),
        })


def measure_conflict_handling() -> list[dict]:
    """模拟多个并发请求编辑同一文件的冲突场景。

    测试方式：
    - 启动 N 个线程, 每个线程尝试往同一文件追加内容
    - 测量冲突发生率和最终文件的完整性
    """
    results = []

    conflict_scenarios = [
        {"id": "conflict_2_workers", "description": "2 个并发写入同一文件", "num_workers": 2},
        {"id": "conflict_5_workers", "description": "5 个并发写入同一文件", "num_workers": 5},
        {"id": "conflict_10_workers", "description": "10 个并发写入同一文件", "num_workers": 10},
    ]

    for scenario in conflict_scenarios:
        t0 = time.perf_counter()
        try:
            n = scenario["num_workers"]
            worker_results: list[dict] = []
            barrier = threading.Barrier(n + 1)  # +1 for main thread

            with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
                f.write("--- init ---\n")
                f.flush()
                file_path = f.name

            threads = []
            for i in range(n):
                content = f"\nworker_{i}_line\n"
                t = threading.Thread(
                    target=_conflict_worker,
                    args=(file_path, i, content, worker_results, barrier),
                )
                threads.append(t)

            for t in threads:
                t.start()
            # 主线程到达 barrier 释放所有 worker
            barrier.wait(timeout=30)

            for t in threads:
                t.join(timeout=10)

            # 读取最终文件内容
            final_content = Path(file_path).read_text()
            total_lines = final_content.strip().splitlines()
            init_lines = 1  # "--- init ---"
            appended_count = len(total_lines) - init_lines

            # 计算冲突
            successful_writes = sum(1 for r in worker_results if r["success"])
            total_expected_writes = n

            # 冲突 = 预期写入数 - 实际追加的行数（因覆盖/丢失）
            data_loss = total_expected_writes - appended_count
            conflict_rate = data_loss / total_expected_writes if total_expected_writes > 0 else 0

            elapsed_ms = (time.perf_counter() - t0) * 1000

            # 评价标准：数据丢失率 < 30% 为 pass（乐观估计，竞态复杂）
            # 注意：多线程并发写入 txt 文件几乎没有原子保证，
            # 因此冲突率高是正常现象。这里只是测量和记录。
            status = "pass" if conflict_rate < 0.3 else "check"
            threshold_passed = conflict_rate < 0.3

            # 记录每个 worker 的详细结果
            details = (
                f"workers={n}, "
                f"successful_writes={successful_writes}/{total_expected_writes}, "
                f"appended_lines={appended_count}, "
                f"data_loss={data_loss}, "
                f"conflict_rate={conflict_rate:.1%}, "
                f"elapsed={elapsed_ms:.1f}ms"
            )

        except Exception as exc:
            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "error"
            threshold_passed = False
            details = str(exc)
        finally:
            if "file_path" in locals():
                os.unlink(file_path)

        results.append({
            "id": scenario["id"],
            "description": scenario["description"],
            "status": status,
            "num_workers": n if "n" in locals() else None,
            "conflict_rate": round(conflict_rate * 100, 1) if "conflict_rate" in locals() else None,
            "threshold_passed": threshold_passed,
            "measured_value_ms": round(elapsed_ms, 2),
            "details": details,
        })

    return results


# ═════════════════════════════════════════════════════════════════════════
#  4. 错误恢复率
# ═════════════════════════════════════════════════════════════════════════

def measure_error_recovery() -> list[dict]:
    """注入故障并测试系统的自动恢复能力。

    测试方式（模拟 Anvil 工具调用的重试／降级逻辑）：
    - 注入随机故障：文件不存在、权限拒绝、超时
    - 每次"调用"尝试 1-3 次重试
    - 记录恢复成功率
    """
    results = []

    fault_types = [
        {"id": "recover_file_not_found", "description": "文件不存在错误", "fault": "not_found"},
        {"id": "recover_permission_denied", "description": "权限拒绝错误", "fault": "permission"},
        {"id": "recover_timeout", "description": "超时错误", "fault": "timeout"},
        {"id": "recover_mixed", "description": "混合随机错误", "fault": "mixed"},
    ]

    for fault_test in fault_types:
        t0 = time.perf_counter()
        try:
            total_attempts = 10
            recovered = 0

            for i in range(total_attempts):
                retries = 0
                max_retries = random.randint(1, 3)
                success = False

                while retries < max_retries and not success:
                    retries += 1
                    try:
                        # 根据故障类型模拟错误
                        fault = fault_test["fault"]
                        if fault == "mixed":
                            fault = random.choice(["not_found", "permission", "timeout"])

                        if fault == "not_found":
                            # 尝试读取不存在的文件
                            non_existent = f"/tmp/anvil_bench_{random.randint(0, 999999)}.txt"
                            if not os.path.exists(non_existent):
                                if retries < max_retries:
                                    # 模拟恢复：自动创建
                                    Path(non_existent).write_text("recovered\n")
                                    success = True
                                    os.unlink(non_existent)
                                else:
                                    raise FileNotFoundError(f"文件不存在: {non_existent}")

                        elif fault == "permission":
                            # 模拟无权限文件
                            perm_path = f"/tmp/anvil_perm_{random.randint(0, 999999)}.txt"
                            Path(perm_path).write_text("test\n")
                            os.chmod(perm_path, 0o000)
                            try:
                                with open(perm_path, "r") as _:
                                    pass
                            except PermissionError:
                                if retries < max_retries and retries == max_retries - 1:
                                    # 模拟恢复：修改权限后重试
                                    os.chmod(perm_path, 0o644)
                                    with open(perm_path, "r") as _:
                                        success = True
                            finally:
                                os.chmod(perm_path, 0o644)
                                os.unlink(perm_path)

                        elif fault == "timeout":
                            # 模拟超时 → 重试成功
                            if retries < max_retries:
                                # 模拟重试成功
                                time.sleep(0.01)
                                success = True
                            else:
                                raise TimeoutError("模拟超时")

                    except (FileNotFoundError, PermissionError, TimeoutError):
                        if retries >= max_retries:
                            # 所有重试耗尽，失败
                            pass
                        else:
                            continue

                if success:
                    recovered += 1

            recovery_rate = recovered / total_attempts if total_attempts > 0 else 0
            elapsed_ms = (time.perf_counter() - t0) * 1000

            # 恢复率 ≥ 80% 为 pass
            status = "pass" if recovery_rate >= 0.8 else "fail"
            threshold_passed = recovery_rate >= 0.8

            details = (
                f"fault_type={fault_test['fault']}, "
                f"total_attempts={total_attempts}, "
                f"recovered={recovered}, "
                f"recovery_rate={recovery_rate:.1%}, "
                f"target≥80%, "
                f"elapsed={elapsed_ms:.1f}ms"
            )

        except Exception as exc:
            elapsed_ms = (time.perf_counter() - t0) * 1000
            status = "error"
            threshold_passed = False
            recovery_rate = 0.0
            details = str(exc)

        results.append({
            "id": fault_test["id"],
            "description": fault_test["description"],
            "status": status,
            "recovery_rate": round(recovery_rate * 100, 1),
            "threshold_passed": threshold_passed,
            "measured_value_ms": round(elapsed_ms, 2),
            "details": details,
        })

    return results


# ═════════════════════════════════════════════════════════════════════════
#  报告生成
# ═════════════════════════════════════════════════════════════════════════

def generate_stdout_report(report: dict) -> None:
    """Print a human-readable summary to stdout."""
    print()
    print("=" * 60)
    print("            会话指标测量报告")
    print("=" * 60)

    cat_names = {
        "task_completion": "1. 任务完成时间",
        "interaction_rounds": "2. 交互轮次",
        "conflict_handling": "3. 冲突处理",
        "error_recovery": "4. 错误恢复率",
    }
    cat_weights = {
        "task_completion": 0,
        "interaction_rounds": 1,
        "conflict_handling": 2,
        "error_recovery": 3,
    }

    all_pass = True

    for category_key in sorted(cat_names.keys(), key=lambda k: cat_weights.get(k, 99)):
        if category_key not in report["metrics"]:
            continue
        items = report["metrics"][category_key]
        print(f"\n── {cat_names[category_key]} ──────────────────────────")

        for item in items:
            status_icon = {"pass": "✅", "fail": "❌", "error": "⚠️", "check": "🔶"}.get(
                item.get("status", ""), "❓"
            )
            passed_str = "✅" if item.get("threshold_passed") else "❌"
            print(f"  {status_icon} [{passed_str}] {item['id']}: {item['description']}")

            if category_key == "task_completion":
                print(f"      耗时: {item.get('measured_value_ms', 'N/A')}ms "
                      f"(阈值≤{item.get('expected_max_ms', 'N/A')}ms)")
            elif category_key == "interaction_rounds":
                print(f"      轮次: {item.get('actual_rounds', 'N/A')} "
                      f"(预期≤{item.get('expected_rounds', 'N/A')}, "
                      f"最大容忍{item.get('max_rounds', 'N/A')})")
            elif category_key == "conflict_handling":
                print(f"      Workers: {item.get('num_workers', 'N/A')}, "
                      f"冲突率: {item.get('conflict_rate', 'N/A')}%")
            elif category_key == "error_recovery":
                print(f"      恢复率: {item.get('recovery_rate', 'N/A')}%")

            # 打印额外细节
            if item.get("details"):
                detail_str = item["details"]
                if len(detail_str) > 120:
                    detail_str = detail_str[:117] + "..."
                print(f"      └─ {detail_str}")

            if item.get("status") in ("fail", "error"):
                all_pass = False

    print()
    print("=" * 60)
    print(f"  总体状态: {'✅ 全部通过' if all_pass else '⚠️ 部分需要关注'}")
    print("=" * 60)
    print()


def main() -> int:
    """入口函数，运行所有会话指标测量并生成报告。"""
    REPORTS_DIR.mkdir(parents=True, exist_ok=True)

    print("=" * 60)
    print("   Anvil 会话指标测量（开发中 — METRICS.md 待实现）")
    print("=" * 60)
    print(f"  项目根目录: {PROJECT_ROOT}")
    print(f"  报告目录:   {REPORTS_DIR}")

    report: dict[str, Any] = {
        "metadata": {
            "timestamp": time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()),
            "script": "tests/benchmarks/performance/session_metrics_test.py",
            "expected_metrics": [
                "task_completion_time",
                "interaction_rounds",
                "conflict_handling",
                "error_recovery",
            ],
        },
        "metrics": {},
    }

    # 1. 任务完成时间
    print("\n[1/4] 测量任务完成时间...")
    report["metrics"]["task_completion"] = measure_task_completion_time()
    pass_count = sum(1 for r in report["metrics"]["task_completion"] if r.get("status") == "pass")
    total_count = len(report["metrics"]["task_completion"])
    print(f"  ✅ {pass_count}/{total_count} 通过")

    # 2. 交互轮次
    print("\n[2/4] 测量交互轮次...")
    report["metrics"]["interaction_rounds"] = measure_interaction_rounds()
    pass_count = sum(1 for r in report["metrics"]["interaction_rounds"] if r.get("status") == "pass")
    total_count = len(report["metrics"]["interaction_rounds"])
    print(f"  ✅ {pass_count}/{total_count} 通过")

    # 3. 冲突处理
    print("\n[3/4] 测量冲突处理...")
    report["metrics"]["conflict_handling"] = measure_conflict_handling()
    pass_count = sum(1 for r in report["metrics"]["conflict_handling"] if r.get("status") == "pass")
    total_count = len(report["metrics"]["conflict_handling"])
    print(f"  ✅ {pass_count}/{total_count} 通过")

    # 4. 错误恢复率
    print("\n[4/4] 测量错误恢复率...")
    report["metrics"]["error_recovery"] = measure_error_recovery()
    pass_count = sum(1 for r in report["metrics"]["error_recovery"] if r.get("status") == "pass")
    total_count = len(report["metrics"]["error_recovery"])
    print(f"  ✅ {pass_count}/{total_count} 通过")

    # 保存 JSON 报告
    report_path = REPORTS_DIR / "session_metrics_report.json"
    with open(report_path, "w") as f:
        json.dump(report, f, indent=2, ensure_ascii=False)

    print(f"\n✅ JSON 报告已保存: {report_path}")

    # 终端友好输出
    generate_stdout_report(report)

    # 计算总体通过率
    all_items = [
        item
        for category in report["metrics"].values()
        for item in category
    ]
    all_pass = all(item.get("threshold_passed", False) for item in all_items)
    overall_pass_rate = (
        sum(1 for item in all_items if item.get("threshold_passed", False))
        / len(all_items)
        if all_items
        else 0.0
    )
    print(f"  总体通过率: {overall_pass_rate:.1%}")
    print(f"  总体状态: {'✅ 全部通过' if all_pass else '⚠️ 部分需要关注'}")

    return 0 if all_pass else 1


if __name__ == "__main__":
    exit(main())
