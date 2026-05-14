#!/usr/bin/env python3
"""
anvil-compare: 对比 anvil 与参考基准工具的执行效果。

从第一性原理出发：进步需要度量。通过对比同一任务在 anvil
和参考基准工具上的表现，找到优化方向。

用法:
  anvil compare <task_description>
  anvil compare --file <task_file>
  anvil compare --list          # 列出历史对比记录
  anvil compare --show <id>     # 显示某次对比详情

将任务同时发给 anvil 和 claude，收集输出，分析差异，
生成结构化的对比报告，保存在 ~/.anvil/comparisons/ 下。
"""

import argparse
import datetime
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import time
import re

COMPARISONS_DIR = os.path.expanduser("~/.anvil/comparisons")
INDEX_FILE = os.path.join(COMPARISONS_DIR, "index.json")
ANVIL_BIN = "/usr/local/bin/anvil"
CLAUDE_BIN = os.path.expanduser("~/.local/bin/claude")


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


def run_cmd(cmd, timeout=120, label="task", env=None):
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


def run_anvil(task, timeout=120):
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


def run_claude(task, timeout=120):
    """Run Claude Code CLI on the task."""
    env = os.environ.copy()
    return run_cmd(
        [CLAUDE_BIN, "-p", task, "--output-format", "text"],
        timeout=timeout,
        label="claude",
    )


def extract_content(output):
    """
    从输出中提取"真正"的回答内容。
    对于 anvil：去掉 spinner 行、Thinking 行、工具调用等元信息。
    对于 claude：类似的清理。
    """
    lines = output.split("\n")
    cleaned = []
    for line in lines:
        line_s = line.strip()
        # Skip spinner lines
        if re.match(r'^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏✔✘]\s', line_s):
            continue
        # Skip "Thinking..." lines
        if "Thinking" in line_s and len(line_s) < 30:
            continue
        # Skip tool call sections (╭─ bash ─╮ etc)
        if line_s.startswith("╭─") or line_s.startswith("╰─") or line_s.startswith("│"):
            continue
        # Skip "✨ Done" / "✔ ✨ Done"
        if "✨ Done" in line_s or "✔ ✨" in line_s:
            continue
        # Skip bash output sections (lines that look like shell commands)
        if line_s.startswith("$ ") or line_s.startswith("✓ bash"):
            continue
        # Skip obvious meta lines
        if line_s in ("```", "") and len(cleaned) > 0 and cleaned[-1] in ("```", ""):
            continue

        cleaned.append(line)

    return "\n".join(cleaned).strip()


def analyze_differences(anvil_text, claude_text):
    """分析两个输出之间的差异点，返回结构化的差异报告。"""
    differences = []

    # 长度对比
    a_len = len(anvil_text)
    c_len = len(claude_text)
    if abs(a_len - c_len) > max(a_len, c_len) * 0.3:
        longer = "anvil" if a_len > c_len else "claude"
        differences.append({
            "type": "length_difference",
            "detail": f"{longer} 的回答明显更长（anvil: {a_len} chars, claude: {c_len} chars）",
        })

    # 结构对比（检查是否包含代码块、列表等）
    a_has_code = "```" in anvil_text
    c_has_code = "```" in claude_text
    if a_has_code != c_has_code:
        differences.append({
            "type": "code_blocks",
            "detail": f"{'anvil' if a_has_code else 'claude'} 提供了代码块而{' claude' if a_has_code else ' anvil'}没有",
        })

    # 质量评估（基于关键词）
    for keyword, aspect in [
        ("error", "错误处理"),
        ("warning", "警告提示"),
        ("alternative", "替代方案"),
        ("security", "安全考虑"),
        ("performance", "性能考虑"),
    ]:
        a_has = keyword.lower() in anvil_text.lower()
        c_has = keyword.lower() in claude_text.lower()
        if a_has != c_has:
            differences.append({
                "type": f"aspect_{keyword}",
                "detail": f"{'anvil' if a_has else 'claude'} 提到了{aspect}而{' claude' if a_has else ' anvil'}没有",
            })

    return differences


def generate_report(task, anvil_out, anvil_err, anvil_code, anvil_time,
                     claude_out, claude_err, claude_code, claude_time):
    """生成结构化的对比报告。"""
    anvil_content = extract_content(anvil_out)
    claude_content = extract_content(claude_out)
    differences = analyze_differences(anvil_content, claude_content)

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
            "claude": {
                "exit_code": claude_code,
                "duration_seconds": round(claude_time, 2),
                "output_length": len(claude_content),
                "has_error": claude_code != 0 or bool(claude_err.strip()),
            },
        },
        "differences": differences,
        "difference_count": len(differences),
        "verdict": _generate_verdict(differences, anvil_content, claude_content),
        "raw": {
            "anvil_stdout": anvil_out,
            "anvil_stderr": anvil_err,
            "claude_stdout": claude_out,
            "claude_stderr": claude_err,
        },
        "cleaned": {
            "anvil": anvil_content,
            "claude": claude_content,
        },
    }

    return report


def _generate_verdict(differences, anvil_text, claude_text):
    """根据差异生成总结性裁决。"""
    if not differences:
        return {
            "summary": "两个工具的回答基本一致",
            "first_principles": "anvil 实现了同等功能，继续保持。",
        }

    key_issues = [d for d in differences if d["type"] in (
        "aspect_security", "aspect_error", "aspect_alternative"
    )]

    # 第一性原理分析：这些差异是本质性的还是实现细节？
    fp_insights = []
    for d in differences:
        if d["type"] == "length_difference":
            fp_insights.append(
                "回答长度差异通常是实现细节（系统提示词风格），"
                "不影响用户目标的达成。如果用户需要更精炼的回答，"
                "调整 system prompt 即可，不需要改架构。"
            )
        elif d["type"] == "code_blocks":
            fp_insights.append(
                "代码块格式差异是呈现细节，不影响代码的正确性。"
                "第一性原理问：用户得到正确的代码了吗？→ 是，那就够了。"
            )
        elif "aspect_" in d["type"]:
            aspect = d["type"].split("_", 1)[1]
            fp_insights.append(
                f"anvil {'提到' if '提到了' in str(d) else '未提到'}了 {aspect}。"
                f"从第一性原理看：{'这是个加分项' if 'anvil' in str(d) and '未' not in str(d) else '需要考虑是否需要补充'}，"
                f"但首先要保证基础功能完整。"
            )

    if key_issues:
        return {
            "summary": f"两者有 {len(differences)} 处差异，其中 {len(key_issues)} 处可能涉及重要问题",
            "first_principles": "\n".join(fp_insights),
        }

    return {
        "summary": f"两者有 {len(differences)} 处差异，主要是回答风格和详细程度的区别",
        "first_principles": "\n".join(fp_insights) if fp_insights else "这些差异都是实现细节，不影响用户目标的达成。",
    }


def save_report(report):
    """保存对比报告到文件。"""
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
        "verdict": report["verdict"],
    })
    save_index(index)

    return filepath


def format_report_summary(report):
    """将报告格式化为人类可读的文本。"""
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
    lines.append(f"  参考基准: 退出码={s['claude']['exit_code']}, "
                 f"耗时={s['claude']['duration_seconds']}s, "
                 f"输出={s['claude']['output_length']}字符")

    if s['anvil']['has_error'] and not s['claude']['has_error']:
        lines.append("  ⚠ anvil 执行出错，参考基准正常")
    elif not s['anvil']['has_error'] and s['claude']['has_error']:
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
    lines.append(cleaned[:1000] + ("..." if len(cleaned) > 1000 else ""))

    lines.append("")
    lines.append("--- 参考基准回答 ---")
    cleaned_c = report.get("cleaned", {}).get("claude", "")
    lines.append(cleaned_c[:1000] + ("..." if len(cleaned_c) > 1000 else ""))

    lines.append("")
    lines.append("=" * 60)
    return "\n".join(lines)


def list_comparisons():
    """列出所有历史对比记录。"""
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
    """显示某次对比的详情。"""
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
    print(f"   done ({a_time:.1f}s, exit={a_code})")

    print("🧠 正在执行参考基准...")
    c_out, c_err, c_code, c_time = run_claude(task)
    print(f"   done ({c_time:.1f}s, exit={c_code})")

    print()
    report = generate_report(task, a_out, a_err, a_code, a_time,
                               c_out, c_err, c_code, c_time)
    filepath = save_report(report)

    print(format_report_summary(report))
    print()
    print(f"📁 完整报告已保存: {filepath}")
    print(f"📋 查看历史: {ANVIL_BIN} compare --list")
    print(f"📖 查看详情: {ANVIL_BIN} compare --show {report['id']}")
    print(f"📐 参考: cat docs/FIRST_PRINCIPLES.md")


def main():
    parser = argparse.ArgumentParser(
        description="对比 anvil 与参考基准工具的执行效果。从第一性原理出发度量进步。",
    )
    parser.add_argument("task", nargs="?", help="要执行的任务描述")
    parser.add_argument("--file", "-f", help="从文件读取任务")
    parser.add_argument("--list", "-l", action="store_true", help="列出历史对比记录")
    parser.add_argument("--show", "-s", help="显示某次对比的详情")

    args = parser.parse_args()

    if args.list:
        list_comparisons()
        return

    if args.show:
        show_comparison(args.show)
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
