# AgyDock（Antigravity 控制台 · Web + 独立桌面版）

面向 Antigravity 的极简本地桌面工具，集成 **Token 用量看板** 与 **隔离代理控制台**，零 `.NET` 依赖、无 AI 风格设计。

---

## 方式一：独立桌面程序（推荐直接发给朋友使用）

已打包为**单文件便携版 Windows 桌面程序**（基于 Tauri v2 + Windows 原生 WebView2，无需安装 Node.js、`.NET` 或任何运行环境，不开放本地端口，无防火墙弹窗）：

- 成品路径：`dist/AgyDock.exe`
- **分发给朋友**：直接把 `AgyDock.exe` 发给朋友，双击即可运行。
- **多来源自动聚合**：启动后自动发现并聚合扫描 Antigravity App（`%USERPROFILE%\.gemini\antigravity\brain`）、`agy CLI`（`%USERPROFILE%\.gemini\antigravity-cli\brain`）与 T3 Code ACP（`%USERPROFILE%\.t3\userdata\providers\antigravity\*\antigravity-acp\brain`）会话目录。
- **桌面端专属功能**：
  - **系统托盘驻留**：点击关闭按钮（`×`）自动隐藏到右下角状态栏托盘不退出（隐藏后自动暂停轮询，几乎零占用）；左键点击托盘图标立即呼出主窗口，右键菜单提供「显示主窗口 / 隐藏到托盘 / 退出程序」；
  - 顶栏一键切换 **用量看板 / 隔离代理** 双模块，完全免去 `.NET 8 Desktop Runtime` 依赖；
  - **隔离代理模块**（原生 Win32 API 实现，完整移植自 `AntigravityTunPanel`）：
    - 仅让 Antigravity 及其 `language_server` 子进程走本地代理（`--proxy-server` + `HTTP_PROXY/HTTPS_PROXY/NODE_USE_ENV_PROXY=1`），不开 TUN、不改系统代理；
    - **独立代理 Antigravity CLI（`agy`）与接入工具（如 T3 Code）**：
      - 自动探测本机 `agy.exe`（包括 `PATH` 及 T3 Code 内置 `antigravity-acp` 版本），支持**打开仅当前窗口生效的隔离代理终端**、一键执行 `agy models` 验证连通性、以及复制 PowerShell / CMD / Bash 代理环境变量命令；
      - 支持**可逆 CLI Proxy Shim**（在 `%LOCALAPPDATA%\AgyDock\cli-shim` 生成包装脚本并前置到用户 `PATH`，不修改原 `agy.exe` 与系统代理/全局环境变量，可一键停用并清理）；
      - 内置**外部工具独立启动与监控**（自动识别 **T3 Code** 并支持添加任意自定义工具）：Electron/Chromium 工具同时注入代理参数与环境变量，普通 CLI 工具仅注入环境变量，使其拉起的 `agy.exe acp` 子进程自动走本地代理；
    - **国内/内网直连绕过（防风控）**：默认自动绕过 `.cn`、国内常用站点/云服务/镜像与局域网私网段（同步生成 Chromium `--proxy-bypass-list` 与 `NO_PROXY`），并支持自定义直连域名与 Clash Global 模式告警，避免从 Antigravity 访问国内站点走海外节点触发风控；
    - 自动读取 Clash Verge 配置与端口探测、启动时自动拉起 Clash Verge、只读检测 Windows 系统代理与 TUN 虚拟网卡；
    - 基于 Win32 `GetExtendedTcpTable` 实时扫描 TCP 连接明细（同步覆盖 `Antigravity`、`language_server`、`agy` CLI 与受监控工具，区分 `走代理`、`TUN Fake-IP`、`公网直连`、`本地 IPC`）；
    - 支持「验收 20s」累计检查并生成验收报告、刷新 DNS、启动参数预览、结束进程与可折叠路径配置；
  - **用量看板桌面增强**：顶栏 **目录** 按钮支持原生文件夹选择并记住配置（按住 `Shift` 或点击「恢复自动多来源聚合」恢复默认）；**导出 CSV** 弹出系统原生「另存为」框。
  - **账号管理（多账号，免重复登录）**：
    - 顶栏「账号管理」页集中展示当前登录账号（支持点击复制邮箱与令牌状态）、已保存账号健康度、Antigravity 运行与隔离代理环境、以及全局最近切换统计；
    - **套餐等级与剩余额度查看**：支持在「用量看板」顶部查看当前账号套餐等级（Free / Google AI Pro / Google AI Ultra / Standard）、AI Credits、各模型池（Gemini Pro / Gemini Flash / Claude & GPT / Image）剩余额度百分比、重置时间与实时倒计时；在「账号管理」页支持一键「刷新全部额度」及逐账号展开模型级额度明细（所有外部查询自动复用隔离代理）；优先自动复用本机活跃的 Antigravity App / `agy` CLI / T3 Code 最新令牌，并支持在「账号管理 → 离线续期」配置本地 DPAPI 加密 OAuth 刷新凭据（或通过 `AGYDOCK_OAUTH_CLIENT_ID` / `AGYDOCK_OAUTH_CLIENT_SECRET` 环境变量）以支持多账号离线自动续期；
    - **添加账号 / 退出登录**：添加前自动备份当前账号 → 关闭 Antigravity → 清除系统登录凭据 → 重新启动等待新账号登录并自动归档；支持「取消并恢复原账号」与「仅停止等待」；退出登录前同样自动备份，绝不丢失已保存凭据；
    - **灵活切换**：支持「切换并重启」与「仅写入凭据（不重启）」两种模式；系统托盘右键菜单同步展示当前账号并支持**托盘一键快速切换**；
    - **账号整理与校验**：支持备注编辑、账号置顶、邮箱/备注实时搜索与多维度排序（默认 / 最近使用 / 切换次数 / 保存时间 / 邮箱）、DPAPI 备份深度校验与一键打开加密存储目录；
    - **安全与可靠性**：凭据来源为 Windows 凭据管理器 `gemini:antigravity`，本地备份采用当前用户作用域 DPAPI 加密，原子写入并保留 `accounts.json.bak` 滚动备份，操作全程加锁防并发覆盖；
    - 若切换后 Antigravity 界面仍显示旧账号，退出 Antigravity 再打开一次即可（Hub 会按当前凭据重新签发登录态）；
    - 仅桌面版支持（Web 版不读写系统凭据）。

### 重新编译桌面版

```powershell
cd AgyDock
npm run build:desktop
```

编译产物位于 `src-tauri\target\release\antigravity-usage-desktop.exe`（可复制到 `dist\AgyDock.exe` 分发）。

---

## 方式二：本地 Web 服务

零第三方依赖，Node.js >= 20 直接运行：

```powershell
cd AgyDock
npm start
```

默认监听 `http://127.0.0.1:3789`，支持自定义端口与日志目录：

```powershell
node server.mjs --port 4000 --dir "C:\Users\<用户名>\.gemini\antigravity\brain"
```

---

## 核心功能

- **多来源聚合与筛选**：同步扫描 Antigravity App、agy CLI 与 T3 Code ACP 会话日志（自动清理 T3 Code 首轮提示词包装标签以还原真实标题），支持来源筛选条、会话来源标签及按来源导出 CSV。
- **Token 总览（统一换算为 M）**：总消耗、Fresh Input / Output 拆分、缓存命中率（`cache_read / (input + cache_read)`）、主会话数与子代理数。
- **Token 消耗趋势**：按日 / 按小时 SVG 堆叠柱状图，点击图例可独立开关 `Output`、`Input`、`Cache Read` 图层。
- **缓存命中率趋势**：分时段命中率柱状图与均值虚线参考线。
- **模型分布**：自动回溯旧版日志与子代理继承模型，点击模型行可全局筛选。
- **主会话与子代理归属**：默认「合并子代理」汇总展示真实主会话总消耗，点击行可展开归属子代理列表及单步 Step 明细；也可一键切换「独立显示」。

## 运行测试

```powershell
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```
