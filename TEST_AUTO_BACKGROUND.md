# 任务：测试 anvil 自动后台判断功能

## 目标
验证 anvil 的自动后台执行判断功能是否正常工作。

## 测试项目

### 1. 静态规则测试
验证以下命令的判断结果：
- `ls -la` → 应判断为前台执行（Never）
- `npm run dev` → 应判断为后台执行（Always）
- `cargo build --release` → 应根据历史判断（IfSlow）

### 2. 历史学习测试
- 执行命令并检查 ~/.anvil/execution_history.json 是否正确记录
- 验证多次执行后是否正确学习

### 3. 日志路径验证
- 检查后台任务日志是否输出到 `/tmp/.anvil/background/`

## 验证方式
1. 查看 execution_history.json 内容
2. 检查后台日志目录
3. 确认判断逻辑正确

## 项目路径
/Users/limiancai/anvil-main/rust
