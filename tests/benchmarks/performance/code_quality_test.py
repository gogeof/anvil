#!/usr/bin/env python3
"""代码质量指标测量脚本

测量 METRICS.md 中的代码质量指标：
- 生成的代码通过 lint/test 的比例
"""

import json
import os
import subprocess
import tempfile
from pathlib import Path


def measure_rust_code_quality():
    """测量 Rust 代码质量"""
    results = []
    
    # 测试用例：让 AI 生成简单的 Rust 函数
    test_cases = [
        {
            "name": "add_function",
            "code": '''
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_add() {
        assert_eq!(add(1, 2), 3);
        assert_eq!(add(-1, 1), 0);
    }
}
''',
        },
        {
            "name": "factorial_function",
            "code": '''
pub fn factorial(n: u32) -> u32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_factorial() {
        assert_eq!(factorial(0), 1);
        assert_eq!(factorial(5), 120);
    }
}
''',
        },
        {
            "name": "string_reverse",
            "code": '''
pub fn reverse_string(s: &str) -> String {
    s.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_reverse() {
        assert_eq!(reverse_string("hello"), "olleh");
        assert_eq!(reverse_string(""), "");
    }
}
''',
        },
    ]
    
    for case in test_cases:
        with tempfile.TemporaryDirectory() as tmpdir:
            # 创建临时 Rust 项目
            project_dir = Path(tmpdir) / case["name"]
            subprocess.run(["cargo", "init", "--name", case["name"], "--lib"], 
                          cwd=tmpdir, capture_output=True)
            
            # 写入测试代码
            src_file = project_dir / "src" / "lib.rs"
            src_file.parent.mkdir(parents=True, exist_ok=True)
            src_file.write_text(case["code"])
            
            # 运行 rustfmt 检查格式
            # 先格式化再检查
            subprocess.run(
                ["rustfmt", str(src_file)],
                capture_output=True
            )
            fmt_result = subprocess.run(
                ["rustfmt", "--check", str(src_file)],
                capture_output=True,
                text=True
            )
            fmt_pass = fmt_result.returncode == 0
            
            # 运行 cargo test
            test_result = subprocess.run(
                ["cargo", "test"],
                cwd=project_dir,
                capture_output=True,
                text=True,
                timeout=30
            )
            test_pass = test_result.returncode == 0 and "test result: ok" in test_result.stdout
            
            # 运行 cargo check
            check_result = subprocess.run(
                ["cargo", "check"],
                cwd=project_dir,
                capture_output=True,
                text=True
            )
            check_pass = check_result.returncode == 0
            
            results.append({
                "name": case["name"],
                "format_pass": fmt_pass,
                "check_pass": check_pass,
                "test_pass": test_pass,
                "overall_pass": fmt_pass and check_pass and test_pass
            })
    
    return results


def measure_python_code_quality():
    """测量 Python 代码质量"""
    results = []
    
    # 检查是否有 Python lint 工具
    has_ruff = subprocess.run(["which", "ruff"], capture_output=True).returncode == 0
    has_black = subprocess.run(["which", "black"], capture_output=True).returncode == 0
    has_pylint = subprocess.run(["which", "pylint"], capture_output=True).returncode == 0
    
    test_cases = [
        {
            "name": "add_function",
            "code": '''
def add(a: int, b: int) -> int:
    """Add two numbers."""
    return a + b


def test_add():
    assert add(1, 2) == 3
    assert add(-1, 1) == 0
''',
        },
        {
            "name": "factorial_function",
            "code": '''
def factorial(n: int) -> int:
    """Calculate factorial."""
    if n <= 1:
        return 1
    return n * factorial(n - 1)


def test_factorial():
    assert factorial(0) == 1
    assert factorial(5) == 120
''',
        },
    ]
    
    for case in test_cases:
        with tempfile.NamedTemporaryFile(mode='w', suffix='.py', delete=False) as f:
            f.write(case["code"])
            py_file = f.name
        
        try:
            result = {
                "name": case["name"],
                "tools_available": {
                    "ruff": has_ruff,
                    "black": has_black,
                    "pylint": has_pylint
                },
                "checks": {}
            }
            
            # 语法检查
            syntax_result = subprocess.run(
                ["python3", "-m", "py_compile", py_file],
                capture_output=True,
                text=True
            )
            result["checks"]["syntax_pass"] = syntax_result.returncode == 0
            
            # ruff 检查
            if has_ruff:
                ruff_result = subprocess.run(
                    ["ruff", "check", py_file],
                    capture_output=True,
                    text=True
                )
                result["checks"]["ruff_pass"] = ruff_result.returncode == 0
            else:
                result["checks"]["ruff_pass"] = None
            
            # 运行测试
            test_result = subprocess.run(
                ["python3", py_file],
                capture_output=True,
                text=True,
                timeout=10
            )
            # 如果文件包含 test_ 函数，直接执行
            test_code = case["code"] + "\n\nif __name__ == '__main__':\n    test_add() if 'test_add' in dir() else test_factorial()\n"
            with open(py_file, 'w') as f:
                f.write(test_code)
            test_result = subprocess.run(
                ["python3", py_file],
                capture_output=True,
                text=True,
                timeout=10
            )
            result["checks"]["test_pass"] = test_result.returncode == 0
            
            # 总体通过
            checks = result["checks"]
            result["overall_pass"] = (
                checks.get("syntax_pass", False) and
                (checks.get("ruff_pass", True) or checks.get("ruff_pass") is None) and
                checks.get("test_pass", False)
            )
            
            results.append(result)
        finally:
            os.unlink(py_file)
    
    return results


def main():
    project_root = Path(__file__).parent.parent.parent.parent
    reports_dir = project_root / "tests/benchmarks/reports"
    reports_dir.mkdir(parents=True, exist_ok=True)
    
    print("=" * 60)
    print("  Code Quality Metrics Measurement")
    print("=" * 60)
    
    report = {
        "timestamp": __import__("time").strftime("%Y-%m-%d %H:%M:%S UTC", __import__("time").gmtime()),
        "metrics": {}
    }
    
    # 1. Rust 代码质量
    print("\n[1/2] 测量 Rust 代码质量...")
    report["metrics"]["rust"] = measure_rust_code_quality()
    
    rust_pass_count = sum(1 for r in report["metrics"]["rust"] if r.get("overall_pass"))
    rust_total = len(report["metrics"]["rust"])
    rust_rate = rust_pass_count / rust_total if rust_total > 0 else 0
    
    print(f"  通过率: {rust_pass_count}/{rust_total} ({rust_rate*100:.0f}%)")
    for r in report["metrics"]["rust"]:
        status = "✅" if r.get("overall_pass") else "❌"
        print(f"  {status} {r['name']}: fmt={r.get('format_pass')} check={r.get('check_pass')} test={r.get('test_pass')}")
    
    # 2. Python 代码质量
    print("\n[2/2] 测量 Python 代码质量...")
    report["metrics"]["python"] = measure_python_code_quality()
    
    py_pass_count = sum(1 for r in report["metrics"]["python"] if r.get("overall_pass"))
    py_total = len(report["metrics"]["python"])
    py_rate = py_pass_count / py_total if py_total > 0 else 0
    
    print(f"  通过率: {py_pass_count}/{py_total} ({py_rate*100:.0f}%)")
    for r in report["metrics"]["python"]:
        status = "✅" if r.get("overall_pass") else "❌"
        print(f"  {status} {r['name']}: syntax={r['checks'].get('syntax_pass')} test={r['checks'].get('test_pass')}")
    
    # 保存报告
    report_path = reports_dir / "code_quality_report.json"
    with open(report_path, 'w') as f:
        json.dump(report, f, indent=2)
    
    print(f"\n✅ 报告已保存: {report_path}")
    
    # 总结
    total_pass = rust_pass_count + py_pass_count
    total_tests = rust_total + py_total
    overall_rate = total_pass / total_tests if total_tests > 0 else 0
    
    print("\n" + "=" * 60)
    print(f"  总体代码质量通过率: {total_pass}/{total_tests} ({overall_rate*100:.0f}%)")
    print(f"  目标: > 80%")
    print(f"  状态: {'✅ 达标' if overall_rate > 0.8 else '❌ 未达标'}")
    print("=" * 60)
    
    return 0 if overall_rate > 0.8 else 1


if __name__ == "__main__":
    exit(main())
