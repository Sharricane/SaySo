# SaySo

按住说话，松开就把转写好的文字粘贴到光标位置。给开发场景用的语音口述工具——尤其适合在终端里用 Claude Code 编程时直接说出需求。

## 核心思路

```
按下快捷键 → 录音
松开快捷键 → Whisper 转写 → LLM 润色 → 写入剪贴板 → 模拟粘贴
```

不做复杂的"模拟打字"，统一走"剪贴板 + 粘贴"。任何接受 Ctrl+V / Cmd+V 的输入框都能用——终端、浏览器、IDE 通吃。

## 目标平台

- [x] Windows 10/11（首要目标，覆盖 WSL2 用户）
- [ ] macOS 13+（Phase 3）
- [ ] Linux 原生（不做。WSL2 用户用 Windows 版即可）

## 主要使用场景

1. 在 Windows Terminal / iTerm2 里用 Claude Code 时口述编程需求
2. 在 claude.ai 等浏览器页面里口述输入

## 技术栈（纯 Rust）

| 模块 | 选型 |
|---|---|
| GUI（设置窗口） | `eframe` + `egui` |
| 系统托盘 | `tray-icon` |
| 全局快捷键 | `rdev` |
| 音频采集 | `cpal` |
| WAV 编码 | `hound` |
| HTTP 客户端 | `reqwest`（rustls） |
| 异步运行时 | `tokio` |
| 剪贴板 | `arboard` |
| 模拟粘贴 | `enigo` |
| 配置序列化 | `serde` + `toml` |
| 错误处理 | `anyhow` + `thiserror` |
| 日志 | `tracing` + `tracing-subscriber` |

整个仓库 **0 行 JS/TS/Python**，单一 Rust crate。

## 默认服务

| 步骤 | 服务 | 模型 | 成本 |
|---|---|---|---|
| STT | Groq | `whisper-large-v3-turbo` | 免费 tier 覆盖（2000 req/天，7200 秒/小时） |
| LLM 润色 | Groq | `llama-3.3-70b-versatile` | 免费 tier 覆盖 |

接口都是 OpenAI 兼容格式，**改配置文件里的 `base_url` 和 `model` 就能换任意兼容服务**（OpenAI / Anthropic / 自部署）。

## 开发环境

在 **WSL2 里写代码、交叉编译到 Windows**，不在 Windows 装 Rust：

```bash
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu

cargo build --target x86_64-pc-windows-gnu --release
./target/x86_64-pc-windows-gnu/release/sayso.exe  # WSL interop 直接以 Windows 进程跑起来
```

## 参考项目

- [Handy](https://github.com/cjpais/Handy) — `cpal` / `rdev` / 状态机参考（其余忽略）
- [whisrs](https://github.com/y0sif/whisrs) — 多后端 ASR 抽象
- [light-whisper](https://github.com/sypsyp97/light-whisper) — LLM 润色服务层设计

## 开发计划

按 [`CLAUDE.md`](./CLAUDE.md) 中定义的 Phase 推进。当前：Phase 1（Windows MVP）。
