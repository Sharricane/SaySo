# SaySo — Claude Code 工作说明

这个文件是后续 Claude Code 会话的工作约束。先读完再动手。

## 项目目标

做一个轻量的语音口述桌面工具：按住快捷键说话，松开后转写、润色、粘贴到当前光标位置。主要服务两个场景：
1. 在终端里用 Claude Code 时直接口述需求
2. 在浏览器里用 claude.ai 时口述输入

## 技术栈（已定，不要更换）

**纯 Rust，单一 crate，0 行其他语言。**

| 模块 | crate |
|---|---|
| GUI（设置窗口） | `eframe` / `egui` |
| 系统托盘 | `tray-icon` |
| 全局快捷键 | `rdev` |
| 音频采集 | `cpal` |
| WAV 编码 | `hound` |
| HTTP | `reqwest`（默认 features 关掉，启 `rustls-tls` + `json` + `multipart`） |
| 异步 | `tokio`（features：`rt-multi-thread`、`macros`、`sync`、`time`） |
| 剪贴板 | `arboard` |
| 模拟键盘粘贴 | `enigo` |
| 序列化 | `serde`、`toml` |
| 错误 | `anyhow`（业务）、`thiserror`（库错误类型） |
| 日志 | `tracing`、`tracing-subscriber` |

## API 抽象

STT 和 LLM 都通过 **OpenAI 兼容接口**调用：

- `Stt` trait：`async fn transcribe(&self, wav: Bytes) -> Result<String>`
- `Llm` trait：`async fn polish(&self, raw: &str) -> Result<String>`

唯一具体实现：`OpenAiCompatible { base_url, model, api_key }`。配置里改 base_url + model 名即可切换任意 OpenAI 兼容服务。

## 默认配置

| 项 | 默认 |
|---|---|
| STT base_url | `https://api.groq.com/openai/v1` |
| STT model | `whisper-large-v3-turbo` |
| LLM base_url | `https://api.groq.com/openai/v1` |
| LLM model | `llama-3.3-70b-versatile` |
| 全局快捷键 | `Right Ctrl` 按住录音、松开停止 |
| 录音格式 | 16kHz / mono / 16-bit PCM → WAV |
| 录音最大时长 | 60 秒（超时自动停止） |
| 润色 system prompt | "Clean up this dictation transcript. Remove filler words, add punctuation, fix obvious errors. Preserve meaning and language. Output only the cleaned text, nothing else." |

## 配置文件位置

- Windows: `%APPDATA%\SaySo\config.toml`
- macOS: `~/Library/Application Support/SaySo/config.toml`
- 首启动若不存在则用默认值生成

**API key 优先级**：环境变量 `GROQ_API_KEY` > 配置文件 `api_key` 字段。配置文件**必须**在 `.gitignore` 内。

## 开发环境（重要）

代码在 **WSL2 (Arch Linux)** 里写，交叉编译到 **Windows MSVC ABI**，**不在 Windows 装 Rust，也不需要系统 mingw**。

```bash
# 一次性安装（全部用户级，无需 root）
cargo install cargo-xwin
rustup target add x86_64-pc-windows-msvc

# 每次编译（.cargo/config.toml 已设默认 target，直接 cargo xwin build 即可）
cargo xwin build --release

# 运行：WSL interop 把它当 Windows 进程跑，能访问 Windows 麦克风 / 快捷键 / 剪贴板
./target/x86_64-pc-windows-msvc/release/sayso.exe
```

**为什么是 xwin 而非 mingw**：mingw-w64 在 Arch 是系统包，要 root 才能装；`cargo-xwin` 是用户级 cargo subcommand，按需下载 MSVC SDK 片段到用户目录。开发机无 root 权限时唯一可行方案，构建产物为标准 PE32+ Windows 可执行文件。

**API key 注入**：开发期把 `GROQ_API_KEY` 放进项目根 `.env`（已 gitignore），`dotenvy::dotenv()` 在 `main()` 入口加载。生产用户走配置文件 / OS keyring。

**测试策略**：单元测试用 `cargo test --target x86_64-unknown-linux-gnu`，避开依赖 Windows API 的模块；集成/手动测试通过运行 .exe。

## 架构原则

- **不做"模拟打字"式文本注入**。所有输出走「写入剪贴板 → 模拟 Ctrl+V / Cmd+V」。
- 核心交互是 **按住-说话-松开**（push-to-talk），不是 toggle。
- API key 等敏感信息**永远不进 git**。
- 录音 PCM 全程在内存中，**不写临时文件**（一次录音 60s × 16kHz × 2byte = 1.92MB，内存完全可放）。
- 注释只写"为什么"，不写"是什么"。

## 模块拆分

```
src/
├── main.rs          入口：起 tokio runtime + tray + 各线程
├── app.rs           中心状态机 (Idle/Recording/Transcribing/Polishing/Pasting)
├── config.rs        配置 load/save（TOML）
├── hotkey.rs        rdev 全局快捷键监听（独立 std::thread，rdev 不支持 async）
├── audio.rs         cpal 录音 → 内存 Vec<i16>
├── encode.rs        PCM → WAV bytes（hound::WavWriter 写到 Cursor）
├── stt.rs           Stt trait + OpenAiCompatible 实现
├── llm.rs           Llm trait + OpenAiCompatible 实现
├── paste.rs         arboard 写剪贴板 + enigo 模拟 Ctrl+V
├── tray.rs          tray-icon + 状态图标切换
└── ui/
    ├── mod.rs
    └── settings.rs  eframe + egui 设置窗口
```

线程模型：
- 主线程：托盘事件循环
- rdev 监听线程：std::thread
- 音频采集线程：cpal 自己起的
- tokio runtime：处理 HTTP 请求
- egui 窗口线程：仅在打开设置窗口时启动

模块间通信用 `tokio::sync::mpsc`，避免 `Arc<Mutex<...>>` 状态泥潭。

## 目标平台

| 平台 | 状态 |
|---|---|
| Windows 10/11 | Phase 1，首要目标 |
| macOS 13+ | Phase 3 |
| Linux 原生 | 不做（WSL2 用户用 Windows 版即可） |

## 开发阶段

每个 Phase 必须独立跑通后再进下一个，**不要超前实现**。

### Phase 1：Windows MVP
1. 项目骨架 + `.cargo/config.toml` 默认 windows-gnu target
2. 系统托盘图标 + 退出菜单
3. rdev 全局快捷键监听（默认 Right Ctrl，按住录音、松开停止）
4. cpal 录音 → 内存 PCM
5. hound 编码 WAV
6. 调 Groq Whisper API 转写
7. arboard 写入剪贴板
8. enigo 模拟一次 Ctrl+V
9. 托盘图标三态：idle / recording / transcribing

**Phase 1 不做**：LLM 润色、设置窗口、剪贴板恢复、历史记录。

### Phase 2：LLM 润色 + 设置窗口
- 调 Groq Llama-3.3-70B 润色（trait 已就位，加实现 + 串到流程）
- egui 设置窗口：base_url / model / api_key / 快捷键 / 润色 prompt
- 剪贴板恢复（粘贴完恢复原内容）
- 简单历史记录（内存中保留最近 N 条）

### Phase 3：macOS 适配
- 权限引导（麦克风 + 辅助功能）
- 签名 / 公证
- 快捷键改 macOS 习惯

### Phase 4：分发
- GitHub Actions 自动构建 Windows .exe（在 WSL2 同款 mingw 路径下）
- macOS .dmg
- 自动更新

## 禁止事项

- ❌ 不要换框架（不许引入 Tauri / Electron / Qt / React / Vue / Python）
- ❌ 不要做 Linux 原生（X11/Wayland）支持
- ❌ 不要打包本地 Whisper / LLM 模型
- ❌ 不要把 API key、`.env`、录音 wav 提交进 git
- ❌ 不要在 Phase 1 加 UI 设置面板或 LLM 润色
- ❌ 不要写多语言注释或多余注释——只在"为什么不显而易见"时写一行

## 参考实现

需要技术决策时，先查这几个仓库的对应代码：

- https://github.com/cjpais/Handy — `cpal`、`rdev`、状态机
- https://github.com/y0sif/whisrs — 多后端 ASR 抽象
- https://github.com/sypsyp97/light-whisper — LLM 润色服务层组织
- https://github.com/emilk/egui — eframe / egui 官方示例
- https://github.com/tauri-apps/tray-icon — tray-icon 独立用法（不依赖 Tauri）
