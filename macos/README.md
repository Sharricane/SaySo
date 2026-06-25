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
- [~] macOS 13+（Phase 3，开发中：核心流程已在 Apple Silicon 原生编译跑通；签名/公证/.app 打包未做）
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

在 **WSL2 里写代码、交叉编译到 Windows**，不在 Windows 装 Rust，不需要 root，不需要装系统 mingw：

```bash
# 一次性安装（全部用户级）
cargo install cargo-zigbuild
rustup target add x86_64-pc-windows-gnu

# 下载 zig 工具链（单文件自包含，~55MB）
curl -L https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz \
  | tar -xJ -C ~/.local
ln -sf ~/.local/zig-x86_64-linux-0.16.0/zig ~/.local/bin/zig

# 编译
cargo zigbuild --release

# 跑（WSL interop 自动以 Windows 进程启动，访问 Windows 麦克风 / 代理）
./target/x86_64-pc-windows-gnu/release/sayso.exe
```

`.env` 文件放项目根（已 gitignore）：
```
GROQ_API_KEY=gsk_xxxxxxxx
# 中国大陆用户：让 reqwest 走本地 Clash
HTTPS_PROXY=http://127.0.0.1:7890
```

## macOS 构建与运行（Phase 3，开发中）

在 Mac 上**原生**编译，不交叉编译。Apple Silicon 开箱即用，Intel 机器把 target 换成 `x86_64-apple-darwin`。

```bash
# 一次性：装 Rust（rustup 默认就带当前主机 target）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 编译 + 运行（.cargo/config.toml 在本分支已不强制 target，默认编原生 Mach-O）
cargo build --release
./target/release/sayso
```

`.env` 放项目根（已 gitignore），跟 Windows 一致：

```
GROQ_API_KEY=gsk_xxxxxxxx
# 需要代理时：HTTPS_PROXY=http://127.0.0.1:7890
```

### 默认快捷键

macOS 默认 PTT 键是**右 Option（⌥）**，按住说话、松开粘贴。换键用环境变量：

```bash
SAYSO_HOTKEY=RightCommand ./target/release/sayso   # 右 ⌘
SAYSO_HOTKEY=Fn ./target/release/sayso             # 地球/Fn 键（新版 macOS 可能被系统截走，不保证稳定）
```

### 系统权限（关键，第一次必做）

SaySo 要全局监听按键、读麦克风、合成 ⌘V，macOS 会拦三类权限。**给"运行 SaySo 的那个终端 App"（Terminal / iTerm2 / VS Code 等）授权**，不是给 sayso 二进制本身：

| 权限 | 位置 | 没授权的表现 |
|---|---|---|
| 麦克风 | 系统设置 → 隐私与安全性 → 麦克风 | 录到的全是静音，转写空白 |
| 输入监控 | 系统设置 → 隐私与安全性 → 输入监控 | 收不到快捷键，程序打印权限提示后退出 |
| 辅助功能 | 系统设置 → 隐私与安全性 → 辅助功能 | ⌘V 合成失败，文字只进剪贴板不粘贴 |

首次运行通常会因为缺"输入监控"直接退出并打印中文提示——按提示勾选后**重启终端**再跑。麦克风权限会在首次录音时弹窗。

> 当前仅验证到「原生编译通过 + 二进制可启动」。完整的录音→转写→粘贴链路依赖上述权限与 `GROQ_API_KEY`，需在你本机授权后手动验证。签名 / 公证 / `.app` 打包尚未做。

## 参考项目

- [Handy](https://github.com/cjpais/Handy) — `cpal` / `rdev` / 状态机参考（其余忽略）
- [whisrs](https://github.com/y0sif/whisrs) — 多后端 ASR 抽象
- [light-whisper](https://github.com/sypsyp97/light-whisper) — LLM 润色服务层设计

## 开发计划

按 [`CLAUDE.md`](./CLAUDE.md) 中定义的 Phase 推进。当前：Phase 1（Windows MVP）。
