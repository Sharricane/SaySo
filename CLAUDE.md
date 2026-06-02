# SaySo — Claude Code 工作说明

这个文件是后续 Claude Code 会话的工作约束。先读完再动手。

## 项目目标

做一个轻量的语音口述桌面工具：按住快捷键说话，松开后转写、润色、粘贴到当前光标位置。主要服务两个场景：
1. 在终端里用 Claude Code 时直接口述需求
2. 在浏览器里用 claude.ai 时口述输入

## 技术栈（已确定，不要更换）

- **Tauri 2.x**（Rust 后端 + Web 前端，跨平台桌面框架）
- **前端**：React + Vite + TypeScript + Tailwind CSS
- **音频**：`cpal` 录麦克风 → 16kHz mono WAV
- **STT**：OpenAI 兼容的 Whisper API（运行时通过设置面板配置 base URL + API key，默认 OpenAI）
- **LLM 润色**：OpenAI 兼容接口（默认 Anthropic Claude，可换 OpenAI/Groq）
- **全局快捷键**：`tauri-plugin-global-shortcut`
- **剪贴板**：`tauri-plugin-clipboard-manager`
- **模拟粘贴**：`enigo`

## 架构原则

- **不做"模拟打字"式文本注入**。所有输出走「写入剪贴板 → 模拟 Ctrl+V / Cmd+V」。
- 核心交互是 **按住-说话-松开**（push-to-talk），不是 toggle。
- API key 等敏感信息**永远不进 git**，存到 OS 安全存储（`tauri-plugin-stronghold` 或系统 keyring）；本地配置文件必须被 `.gitignore` 排除。
- 录音文件用完即删，不持久化到磁盘（除非用户主动开启历史功能）。
- 注释只写"为什么"，不写"是什么"。

## 目标平台

| 平台 | 状态 |
|---|---|
| Windows 10/11 | Phase 1，首要目标 |
| macOS 13+ | Phase 3 |
| Linux 原生 | 不做（WSL2 用户用 Windows 版即可） |

## 开发阶段

每个 Phase 必须独立跑通后再进下一个，**不要超前实现**。

### Phase 1：Windows MVP（最小可用链路）
1. Tauri 项目骨架 + 系统托盘图标
2. 全局快捷键监听（默认 F5，按住录音、松开停止）
3. cpal 录音 → 临时 WAV 文件（用完删除）
4. 调 Whisper API 转写
5. 把转写文本写入剪贴板
6. enigo 模拟一次 Ctrl+V
7. 托盘图标显示三态：idle / recording / transcribing

**Phase 1 不做**：润色、设置面板、剪贴板恢复、历史记录。

### Phase 2：润色 + 设置面板
- LLM 润色（去口癖、加标点、保持原意）
- 设置 UI：API key、快捷键、模型、润色 prompt 自定义
- 剪贴板恢复（用完恢复原内容）
- 历史记录（本地 SQLite，可关闭）

### Phase 3：macOS 适配
- 权限引导（麦克风 + 辅助功能）
- 签名 / 公证流程
- 快捷键改用 macOS 习惯（Cmd 系，避开系统占用）

### Phase 4：分发
- GitHub Actions 自动打包 Windows `.exe` + macOS `.dmg`
- 自动更新机制

## 参考实现

需要技术决策时，先查这几个仓库的对应代码：
- https://github.com/xarthurx/whisperi — Tauri + 云端 API，整体架构最贴近本项目
- https://github.com/fiorelorenzo/local-dictation-app — Tauri 项目目录结构、状态机
- https://github.com/moinulmoin/voicetypr — UI 风格

## 禁止事项

- ❌ 不要做 Linux 原生（X11/Wayland）支持
- ❌ 不要打包本地 Whisper 模型
- ❌ 不要把 API key、`.env`、录音 wav 提交进 git
- ❌ 不要在 Phase 1 里加 UI 设置面板
- ❌ 不要换框架（不要改用 Electron / Qt / Python）
