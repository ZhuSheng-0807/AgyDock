# AgyDock

Antigravity 本地控制台，集成 Token 用量看板、应用隔离代理和多账号管理。

## 功能

- **用量看板**：自动扫描本机已安装的 Antigravity、CLI 与 T3 Code 会话日志；支持自定义目录、按来源与日期区间筛选、趋势图拖拽框选时段、输出速度、缓存命中率、热力日历、模型分布、子代理展开与 CSV 导出。
- **隔离代理**：针对 Antigravity、T3 Code、自定义应用及 `agy` CLI 注入进程级代理参数与环境变量，内置国内及内网直连规则、Clash Verge 端口探测、实时 TCP 连接分类与 20 秒验收；支持 Gemini Desktop 自动启停系统代理与连接监控。
- **账号与额度**：基于 Windows 凭据管理器管理多账号保存、切换、登出与托盘切号，本地凭据采用 DPAPI 加密；支持查看套餐等级、剩余额度与重置倒计时，支持可选 OAuth 离线续期。

## 开发与构建

### 桌面版

```powershell
npm run build:desktop
```

产物：`src-tauri\target\release\antigravity-usage-desktop.exe`

### Web 版

Node.js >= 20，仅包含用量看板：

```powershell
npm start
```

默认监听 `http://127.0.0.1:3789`，支持 `--port`、`--host`、`--dir`：

```powershell
node server.mjs --port 4000 --dir "C:\Users\<用户名>\.gemini\antigravity\brain"
```

### 测试

```powershell
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```
