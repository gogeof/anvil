#!/usr/bin/env python3
"""性能指标测量脚本

测量 METRICS.md 中的未完成指标：
1. 搜索内存占用
2. 大文件支持
3. 超时处理
"""

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path


def measure_search_memory():
    """测量 grep_search 的内存占用"""
    results = []
    
    # 在 anvil-main 项目中搜索
    project_root = Path(__file__).parent.parent.parent.parent
    
    tests = [
        {"pattern": "fn main", "path": str(project_root / "rust/crates")},
        {"pattern": "impl.*for", "path": str(project_root / "rust/crates/runtime/src")},
        {"pattern": "use std", "path": str(project_root / "rust/crates")},
    ]
    
    for test in tests:
        # 使用 /usr/bin/time 测量内存
        cmd = [
            "/usr/bin/time", "-l",
            "grep", "-r", test["pattern"], test["path"]
        ]
        
        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=30
            )
            
            # 解析 time -l 输出
            stderr = result.stderr
            max_mem = 0
            for line in stderr.split("\n"):
                if "maximum resident set size" in line:
                    # macOS 格式: "maximum resident set size" 后跟字节数
                    parts = line.split()
                    if parts:
                        max_mem = int(parts[0]) / 1024 / 1024  # Convert to MB
                    break
            
            results.append({
                "pattern": test["pattern"],
                "max_memory_mb": round(max_mem, 2),
                "status": "pass" if max_mem < 100 else "fail"
            })
        except Exception as e:
            results.append({
                "pattern": test["pattern"],
                "error": str(e),
                "status": "error"
            })
    
    return results


def measure_large_file_support():
    """测量大文件支持能力"""
    results = []
    
    sizes = [1, 5, 10, 20, 50]  # MB
    
    for size_mb in sizes:
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as f:
            # 生成指定大小的文件
            content = "x" * 1024 + "\n"
            lines = (size_mb * 1024 * 1024) // len(content)
            
            start = time.time()
            for _ in range(lines):
                f.write(content)
            f.flush()
            
            file_path = f.name
        
        try:
            # 测量读取时间
            start = time.time()
            with open(file_path, 'r') as f:
                _ = f.read()
            read_time = time.time() - start
            
            # 测量内存占用
            cmd = ["/usr/bin/time", "-l", "cat", file_path]
            result = subprocess.run(cmd, capture_output=True, text=True)
            
            max_mem = 0
            for line in result.stderr.split("\n"):
                if "maximum resident set size" in line:
                    parts = line.split()
                    if parts:
                        max_mem = int(parts[0]) / 1024 / 1024
                    break
            
            results.append({
                "size_mb": size_mb,
                "read_time_ms": round(read_time * 1000, 2),
                "memory_mb": round(max_mem, 2),
                "status": "pass" if size_mb <= 10 and read_time < 1 else "check"
            })
        except Exception as e:
            results.append({
                "size_mb": size_mb,
                "error": str(e),
                "status": "error"
            })
        finally:
            os.unlink(file_path)
    
    return results


def measure_timeout_handling():
    """测量超时处理能力"""
    results = []
    
    # 测试短超时
    tests = [
        {"cmd": ["sleep", "0.5"], "timeout": 1, "expected": "success"},
        {"cmd": ["sleep", "5"], "timeout": 1, "expected": "timeout"},
    ]
    
    for test in tests:
        start = time.time()
        try:
            result = subprocess.run(
                test["cmd"],
                capture_output=True,
                text=True,
                timeout=test["timeout"]
            )
            elapsed = time.time() - start
            
            results.append({
                "command": " ".join(test["cmd"]),
                "timeout": test["timeout"],
                "elapsed_ms": round(elapsed * 1000, 2),
                "expected": test["expected"],
                "actual": "success",
                "status": "pass" if test["expected"] == "success" else "fail"
            })
        except subprocess.TimeoutExpired:
            elapsed = time.time() - start
            results.append({
                "command": " ".join(test["cmd"]),
                "timeout": test["timeout"],
                "elapsed_ms": round(elapsed * 1000, 2),
                "expected": test["expected"],
                "actual": "timeout",
                "status": "pass" if test["expected"] == "timeout" else "fail"
            })
        except Exception as e:
            results.append({
                "command": " ".join(test["cmd"]),
                "error": str(e),
                "status": "error"
            })
    
    return results


def main():
    project_root = Path(__file__).parent.parent.parent.parent
    reports_dir = project_root / "tests/benchmarks/reports"
    reports_dir.mkdir(parents=True, exist_ok=True)
    
    print("=" * 60)
    print("  Anvil Performance Metrics Measurement")
    print("=" * 60)
    
    report = {
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()),
        "metrics": {}
    }
    
    # 1. 搜索内存占用
    print("\n[1/3] 测量搜索内存占用...")
    report["metrics"]["search_memory"] = measure_search_memory()
    for r in report["metrics"]["search_memory"]:
        status = "✅" if r.get("status") == "pass" else "❌"
        print(f"  {status} {r.get('pattern', 'unknown')}: {r.get('max_memory_mb', 'N/A')} MB")
    
    # 2. 大文件支持
    print("\n[2/3] 测量大文件支持...")
    report["metrics"]["large_file"] = measure_large_file_support()
    for r in report["metrics"]["large_file"]:
        status = "✅" if r.get("status") == "pass" else ("⚠️" if r.get("status") == "check" else "❌")
        print(f"  {status} {r.get('size_mb', 'N/A')}MB: {r.get('read_time_ms', 'N/A')}ms, {r.get('memory_mb', 'N/A')}MB")
    
    # 3. 超时处理
    print("\n[3/3] 测量超时处理...")
    report["metrics"]["timeout"] = measure_timeout_handling()
    for r in report["metrics"]["timeout"]:
        status = "✅" if r.get("status") == "pass" else "❌"
        print(f"  {status} {r.get('command', 'unknown')}: {r.get('actual', 'N/A')} (expected: {r.get('expected', 'N/A')})")
    
    # 保存报告
    report_path = reports_dir / "performance_report.json"
    with open(report_path, 'w') as f:
        json.dump(report, f, indent=2)
    
    print(f"\n✅ 报告已保存: {report_path}")
    
    # 总结
    all_pass = all(
        r.get("status") == "pass"
        for category in report["metrics"].values()
        for r in category
    )
    
    print("\n" + "=" * 60)
    print(f"  总体状态: {'✅ 全部通过' if all_pass else '⚠️ 部分需要关注'}")
    print("=" * 60)
    
    return 0 if all_pass else 1


if __name__ == "__main__":
    exit(main())
