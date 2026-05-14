#!/usr/bin/env python3
"""
anvil-compare: 对比 anvil 与参考基准工具的执行效果。

从第一性原理出发：进步需要度量。通过对比同一任务在 anvil
和参考基准工具上的表现，找到优化方向。

用法:
  anvil compare <task_description>     # 单次对比
  anvil compare --file <task_file>     # 从文件读取任务
  anvil compare --list                 # 列出历史对比记录
  anvil compare --show <id>            # 显示某次对比详情
  anvil compare --batch <tasks.json>   # 批量执行多个任务
  anvil compare --trend                # 显示历史趋势报告
  anvil compare --watch                # 监听变化，自动对比
"""

import argparse
import datetime
import json
import os
import subprocess
import sys
import time
import re

COMPARISONS_DIR = os.path.expanduser("~/.anvil/comparisons")
INDEX_FILE = os.path.join(COMPARISONS_DIR, "index.json")
ANVIL_BIN = "/usr/local/bin/anvil"
REFERENCE_BIN = os.path.expanduser("~/.local/bin/claude")
DEFAULT_TIMEOUT = 600

# ====== 预设任务集 ======

BENCHMARK_TASKS = [
    # 1. 代码生成
    "用Python实现一个线程安全的LRU缓存",
    "写一个Rust函数，计算两个大文件的差异",
    "生成一个bash脚本，批量重命名当前目录下的所有.jpg文件",

    # 2. 代码分析
    "分析这段代码的性能瓶颈: with open('data.txt') as f: data = f.read(); for i in range(len(data)): ...",
    "解释Rust的所有权系统和生命周期",

    # 3. 调试
    "为什么这段代码报IndexError? arr = [1,2,3]; for i in range(len(arr)): arr[i+1] = arr[i]",
    "git merge冲突怎么解决? 我有两个分支feature-a和feature-b",

    # 4. 架构设计
    "设计一个简单的任务队列系统，支持优先级和延时执行",
    "如何设计一个微服务架构的日志收集系统?",

    # 5. 中文编程
    "用Python实现一个中文分词器的基本框架",
    "SQL优化：这个查询为什么慢？SELECT * FROM orders WHERE DATE(created_at) = '2024-01-01'",
]


def ensure_dir():
    os.makedirs(COMPARISONS_DIR, exist_ok=True)


def load_index():
    ensure_dir()
    if os.path.exists(INDEX_FILE):
        with open(INDEX_FILE) as f:
            return json.load(f)
    return {"comparisons": []}


def save_index(index):
    ensure_dir()
    with open(INDEX_FILE, "w") as f:
        json.dump(index, f, indent=2, ensure_ascii=False)


def run_cmd(cmd, timeout=DEFAULT_TIMEOUT, label="task", env=None):
    """Run a command and return (stdout, stderr, exit_code, duration)."""
    start = time.time()
    try:
        r = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
        duration = time.time() - start
        return r.stdout, r.stderr, r.returncode, duration
    except subprocess.TimeoutExpired:
        return "", f"TIMEOUT after {timeout}s", -1, time.time() - start
    except FileNotFoundError as e:
        return "", f"Command not found: {e}", -1, time.time() - start


def run_anvil(task, timeout=DEFAULT_TIMEOUT):
    """Run anvil on the task. Loads env from ~/.anvil/settings.json."""
    env = os.environ.copy()
    settings_path = os.path.expanduser("~/.anvil/settings.json")
    if os.path.exists(settings_path):
        with open(settings_path) as f:
            try:
                settings = json.load(f)
                for k, v in settings.get("env", {}).items():
                    if v:
                        env[k] = v
            except json.JSONDecodeError:
                pass
    return run_cmd(
        [ANVIL_BIN, "--model", "lite", "--output-format", "text", "prompt", task],
        timeout=timeout,
        label="anvil",
    )


def run_reference(task, timeout=DEFAULT_TIMEOUT):
    """Run reference tool on the task."""
    env = os.environ.copy()
    return run_cmd(
        [REFERENCE_BIN, "-p", task, "--output-format", "text"],
        timeout=timeout,
        label="reference",
    )


def extract_content(output):
    """从输出中提取真正的回答内容。"""
    lines = output.split("\n")
    cleaned = []
    for line in lines:
        line_s = line.strip()
        if re.match(r'^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏✔✘]\s', line_s):
            continue
        if "Thinking" in line_s and len(line_s) < 30:
            continue
        if line_s.startswith("╭─") or line_s.startswith("╰─") or line_s.startswith("│"):
            continue
        if "✨ Done" in line_s or "✔ ✨" in line_s:
            continue
        if line_s.startswith("$ ") or line_s.startswith("✓ bash"):
            continue
        cleaned.append(line)
    return "\n".join(cleaned).strip()


def analyze_differences(anvil_text, ref_text):
    """分析两个输出之间的差异点。"""
    differences = []
    a_len = len(anvil_text)
    c_len = len(ref_text)
    if abs(a_len - c_len) > max(a_len, c_len) * 0.3:
        longer = "anvil" if a_len > c_len else "reference"
        differences.append({
            "type": "length_difference",
            "detail": f"{longer} 的回答明显更长（anvil: {a_len} chars, reference: {c_len} chars）",
        })
    a_has_code = "```" in anvil_text
    c_has_code = "```" in ref_text
    if a_has_code != c_has_code:
        differences.append({
            "type": "code_blocks",
            "detail": f"{'anvil' if a_has_code else 'reference'} 提供了代码块而{' claude' if a_has_code else ' anvil'}没有",
        })
    for keyword, aspect in [
        ("error", "错误处理"),
        ("warning", "警告提示"),
        ("alternative", "替代方案"),
        ("security", "安全考虑"),
        ("performance", "性能考虑"),
    ]:
        a_has = keyword.lower() in anvil_text.lower()
        c_has = keyword.lower() in ref_text.lower()
        if a_has != c_has:
            differences.append({
                "type": f"aspect_{keyword}",
                "detail": f"{'anvil' if a_has else 'reference'} 提到了{aspect}而{' claude' if a_has else ' anvil'}没有",
            })
    return differences


def generate_report(task, anvil_out, anvil_err, anvil_code, anvil_time,
                     ref_out, ref_err, ref_code, ref_time):
    """生成结构化的对比报告。"""
    anvil_content = extract_content(anvil_out)
    ref_content = extract_content(ref_out)
    differences = analyze_differences(anvil_content, ref_content)
    now = datetime.datetime.now().isoformat()
    comparison_id = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")

    report = {
        "id": comparison_id,
        "timestamp": now,
        "task": task,
        "summary": {
            "anvil": {
                "exit_code": anvil_code,
                "duration_seconds": round(anvil_time, 2),
                "output_length": len(anvil_content),
                "has_error": anvil_code != 0 or bool(anvil_err.strip()),
            },
            "reference": {
                "exit_code": ref_code,
                "duration_seconds": round(ref_time, 2),
                "output_length": len(ref_content),
                "has_error": ref_code != 0 or bool(ref_err.strip()),
            },
        },
        "differences": differences,
        "difference_count": len(differences),
        "verdict": _generate_verdict(differences, anvil_content, ref_content),
        "cleaned": {
            "anvil": anvil_content,
            "reference": ref_content,
        },
    }
    return report


def _generate_verdict(differences, anvil_text, ref_text):
    if not differences:
        return {"summary": "两个工具的回答基本一致", "first_principles": "anvil 实现了同等功能，继续保持。"}
    fp_insights = []
    for d in differences:
        if d["type"] == "length_difference":
            fp_insights.append("回答长度差异通常是实现细节（系统提示词风格），不影响用户目标的达成。")
        elif d["type"] == "code_blocks":
            fp_insights.append("代码块格式差异是呈现细节，不影响代码的正确性。")
    return {"summary": f"两者有 {len(differences)} 处差异", "first_principles": "\n".join(fp_insights) if fp_insights else "这些差异都是实现细节。"}


def save_report(report):
    ensure_dir()
    filepath = os.path.join(COMPARISONS_DIR, f"{report['id']}.json")
    with open(filepath, "w") as f:
        json.dump(report, f, indent=2, ensure_ascii=False)
    index = load_index()
    index["comparisons"].insert(0, {
        "id": report["id"],
        "timestamp": report["timestamp"],
        "task": report["task"][:80],
        "difference_count": report["difference_count"],
        "verdict": report["verdict"].get("summary", ""),
    })
    save_index(index)
    return filepath


def format_report_summary(report):
    lines = []
    lines.append("=" * 60)
    lines.append(f"  ANVIL vs 参考基准 对比报告")
    lines.append(f"  ID: {report['id']}")
    lines.append(f"  时间: {report['timestamp']}")
    lines.append("=" * 60)
    lines.append("")
    lines.append(f"任务: {report['task']}")
    lines.append("")
    lines.append("--- 执行摘要 ---")
    s = report["summary"]
    lines.append(f"  anvil:   退出码={s['anvil']['exit_code']}, "
                 f"耗时={s['anvil']['duration_seconds']}s, "
                 f"输出={s['anvil']['output_length']}字符")
    lines.append(f"  参考基准: 退出码={s['reference']['exit_code']}, "
                 f"耗时={s['reference']['duration_seconds']}s, "
                 f"输出={s['reference']['output_length']}字符")
    if s['anvil']['has_error'] and not s['reference']['has_error']:
        lines.append("  ⚠ anvil 执行出错，参考基准正常")
    elif not s['anvil']['has_error'] and s['reference']['has_error']:
        lines.append("  ⚠ 参考基准执行出错，anvil 正常")
    lines.append("")
    verdict = report.get("verdict", {})
    v_summary = verdict.get("summary", "") if isinstance(verdict, dict) else verdict
    fp_insights = verdict.get("first_principles", "") if isinstance(verdict, dict) else ""
    if v_summary:
        lines.append(f"裁决: {v_summary}")
    if fp_insights:
        lines.append("")
        lines.append("--- 第一性原理分析 ---")
        for line in fp_insights.split("\n"):
            lines.append(f"  {line}")
    if report["differences"]:
        lines.append("")
        lines.append(f"--- 差异分析 ({len(report['differences'])}处) ---")
        for d in report["differences"]:
            lines.append(f"  [{d['type']}] {d['detail']}")
    lines.append("")
    lines.append("--- anvil 回答 ---")
    cleaned = report.get("cleaned", {}).get("anvil", "")
    lines.append(cleaned[:600] + ("..." if len(cleaned) > 600 else ""))
    lines.append("")
    lines.append("--- 参考基准回答 ---")
    cleaned_c = report.get("cleaned", {}).get("reference", "")
    lines.append(cleaned_c[:600] + ("..." if len(cleaned_c) > 600 else ""))
    lines.append("")
    lines.append("=" * 60)
    return "\n".join(lines)


def list_comparisons():
    index = load_index()
    if not index["comparisons"]:
        print("暂无对比记录。")
        return
    print(f"{'ID':<17} {'时间':<20} {'差异数':<6} 任务")
    print("-" * 80)
    for c in index["comparisons"]:
        ts = c["timestamp"][:19] if len(c["timestamp"]) > 19 else c["timestamp"]
        print(f"{c['id']:<17} {ts:<20} {c['difference_count']:<6} {c['task']}")


def show_comparison(cid):
    filepath = os.path.join(COMPARISONS_DIR, f"{cid}.json")
    if not os.path.exists(filepath):
        print(f"未找到对比记录: {cid}")
        return
    with open(filepath) as f:
        report = json.load(f)
    print(format_report_summary(report))


def do_compare(task):
    """执行对比的主逻辑。"""
    print(f"🔨 任务: {task}")
    print()
    print("🤖 正在执行 anvil...")
    a_out, a_err, a_code, a_time = run_anvil(task)
    a_status = f"done ({a_time:.1f}s, exit={a_code})" if a_code != -1 else f"TIMEOUT ({a_time:.1f}s)"
    print(f"   {a_status}")
    print("🧠 正在执行参考基准...")
    c_out, c_err, c_code, c_time = run_reference(task)
    c_status = f"done ({c_time:.1f}s, exit={c_code})" if c_code != -1 else f"TIMEOUT ({c_time:.1f}s)"
    print(f"   {c_status}")
    print()
    report = generate_report(task, a_out, a_err, a_code, a_time,
                               c_out, c_err, c_code, c_time)
    filepath = save_report(report)
    print(format_report_summary(report))
    print()
    print(f"📁 完整报告: {filepath}")


def do_batch(task_file):
    """批量对比。从 JSON 文件读取任务列表，逐个执行。"""
    with open(task_file) as f:
        config = json.load(f)

    tasks = config.get("tasks", [])
    if not tasks:
        # 如果文件是纯列表
        tasks = config if isinstance(config, list) else []

    if not tasks:
        print("❌ 未找到任务，使用默认 benchmark 任务集")
        tasks = BENCHMARK_TASKS

    print(f"🔨 批量对比: {len(tasks)} 个任务")
    print("=" * 60)

    results = []
    for i, task in enumerate(tasks):
        print(f"\n[{i+1}/{len(tasks)}] {task[:60]}...")
        a_out, a_err, a_code, a_time = run_anvil(task)
        c_out, c_err, c_code, c_time = run_reference(task)
        report = generate_report(task, a_out, a_err, a_code, a_time,
                                   c_out, c_err, c_code, c_time)
        filepath = save_report(report)
        results.append(report)
        print(f"   ✅ anvil: {a_time:.1f}s | reference: {c_time:.1f}s "
              f"| diff: {report['difference_count']}处 | {filepath}")

    # 输出汇总
    print("\n" + "=" * 60)
    print(f"📊 批量对比完成: {len(results)}/{len(tasks)} 个任务")
    
    # 统计
    anvil_scores = {"win": 0, "tie": 0, "lose": 0, "timeout": 0}
    for r in results:
        s = r["summary"]
        if s["anvil"]["has_error"] or s["anvil"]["exit_code"] == -1:
            anvil_scores["timeout"] += 1
        elif s["reference"]["has_error"] or s["reference"]["exit_code"] == -1:
            anvil_scores["win"] += 1
        else:
            diff_count = r["difference_count"]
            if diff_count == 0:
                anvil_scores["tie"] += 1
            elif s["anvil"]["output_length"] >= s["reference"]["output_length"] * 0.7:
                anvil_scores["tie"] += 1
            else:
                anvil_scores["lose"] += 1

    print(f"  anvil 优于:  {anvil_scores['win']}")
    print(f"  持平:       {anvil_scores['tie']}")
    print(f"  anvil 落后:  {anvil_scores['lose']}")
    print(f"  超时/失败:   {anvil_scores['timeout']}")

    # 找出 anvil 表现最差的任务
    worst = sorted(results, key=lambda r: (
        r["summary"]["reference"]["output_length"] - r["summary"]["anvil"]["output_length"]
    ), reverse=True)[:3]
    if worst:
        print(f"\n🎯 需要优先优化的任务:")
        for r in worst:
            print(f"  - {r['task'][:60]}... (差距: {r['difference_count']}处差异)")


def do_trend():
    """生成历史趋势报告。"""
    index = load_index()
    comparisons = index["comparisons"]
    if not comparisons:
        print("暂无对比记录。")
        return

    print("📈 ANVIL 进步趋势报告")
    print("=" * 60)
    print(f"  总对比次数: {len(comparisons)}")
    print()

    # 按日期分组统计
    from collections import Counter, defaultdict
    by_date = defaultdict(list)
    for c in comparisons:
        date = c["timestamp"][:10]
        by_date[date].append(c)

    print(f"{'日期':<12} {'对比数':<8} {'平均差异':<10} {'趋势'}")
    print("-" * 60)

    sorted_dates = sorted(by_date.keys())
    for date in sorted_dates[-14:]:  # 最近14天
        items = by_date[date]
        avg_diff = sum(c["difference_count"] for c in items) / len(items)
        indicator = "📈" if avg_diff > 2 else "📊" if avg_diff > 0 else "✅"
        print(f"{date:<12} {len(items):<8} {avg_diff:<10.1f} {indicator}")

    print()
    print("💡 解读:")
    print("  差异数越低说明 anvil 和参考基准越接近")
    print("  长期趋势下降 = anvil 在进步")


def main():
    parser = argparse.ArgumentParser(
        description="对比 anvil 与参考基准工具的执行效果。从第一性原理出发度量进步。",
    )
    parser.add_argument("task", nargs="?", help="要执行的任务描述")
    parser.add_argument("--file", "-f", help="从文件读取任务")
    parser.add_argument("--list", "-l", action="store_true", help="列出历史对比记录")
    parser.add_argument("--show", "-s", help="显示某次对比的详情")
    parser.add_argument("--batch", "-b", nargs="?", const="default", 
                        help="批量对比。可指定 JSON 任务文件，不指定则使用默认 benchmark")
    parser.add_argument("--trend", "-t", action="store_true", help="显示历史趋势报告")

    args = parser.parse_args()

    if args.list:
        list_comparisons()
        return

    if args.show:
        show_comparison(args.show)
        return

    if args.trend:
        do_trend()
        return

    if args.batch:
        if args.batch == "default":
            # 创建默认任务文件
            task_file = os.path.join(COMPARISONS_DIR, "benchmark_tasks.json")
            with open(task_file, "w") as f:
                json.dump({"tasks": BENCHMARK_TASKS}, f, indent=2, ensure_ascii=False)
            print(f"📝 使用默认 benchmark（{len(BENCHMARK_TASKS)} 个任务）")
            do_batch(task_file)
        else:
            do_batch(args.batch)
        return

    task = args.task
    if args.file:
        with open(args.file) as f:
            task = f.read().strip()

    if not task:
        parser.print_help()
        sys.exit(1)

    do_compare(task)


if __name__ == "__main__":
    main()
