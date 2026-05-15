# ANVIL Metrics 测试任务

本目录包含用于验证 anvil 性能的测试任务集。

## 目录结构

```
tests/benchmarks/
├── search/           # 搜索能力测试
│   └── latency.json
├── write/            # 编写能力测试
│   └── edit_accuracy.json
├── execute/          # 执行能力测试
│   └── long_running.json
└── humaneval/        # HumanEval 基准
    └── tasks.json
```

## 成功指标

| 维度 | 指标 | 目标值 |
|------|------|--------|
| 搜索 | 延迟 | < 50ms |
| 编写 | 编辑成功率 | > 90% |
| 执行 | 后台任务支持 | ✅ |
| 整体 | HumanEval 完成率 | > 75% |
