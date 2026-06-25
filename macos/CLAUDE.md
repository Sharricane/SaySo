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

代码在 **WSL2 (Arch Linux)** 里写，交叉编译到 **Windows GNU ABI**，**全部用户级，不需要 root，不需要 mingw 系统包，不需要 clang**。

```bash
# 一次性安装（用户级 cargo subcommand）
cargo install cargo-zigbuild
rustup target add x86_64-pc-windows-gnu

# 下载 zig 工具链（单文件二进制，自包含，含 clang）
curl -L https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz \
  | tar -xJ -C ~/.local
ln -sf ~/.local/zig-x86_64-linux-0.16.0/zig ~/.local/bin/zig

# 每次编译（.cargo/config.toml 已设默认 target，直接 cargo zigbuild）
cargo zigbuild --release

# 运行：WSL interop 当 Windows 进程跑，访问 Windows 麦克风 / 快捷键 / 剪贴板
./target/x86_64-pc-windows-gnu/release/sayso.exe
```

**工具链选择历程**：先试 mingw（系统包，需 root，pass）；再试 cargo-xwin（用户级 MSVC SDK，但需要 clang-cl，依赖系统 clang，pass）；最后定型 cargo-zigbuild + Zig（Zig 单文件自带 clang，完全用户级）。

**API key 注入**：开发期把 `GROQ_API_KEY` 放进项目根 `.env`（已 gitignore），`dotenvy::dotenv()` 在 `main()` 入口加载。中国用户在 .env 里加 `HTTPS_PROXY=http://127.0.0.1:7890`（reqwest 走本地 Clash）。

**测试策略**：单元测试 `cargo check --target x86_64-unknown-linux-gnu` 避开 Windows-only crate；集成 / 手动测试通过运行 .exe。

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
| macOS 13+ | Phase 3，开发中（`macos` 分支：核心已原生编译跑通，分发未做） |
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
  - system prompt 要点：输出语言与输入一致；保留技术专有名词（React、Whisper、API…）；不扩写、不增加开场白；直接输出干净文本
- egui 设置窗口：base_url / model / api_key / 快捷键 / 润色 prompt 自定义
- **静音灵敏度可调**：滑块控制 `Recording::is_silent` 阈值（默认 500/i16，~-36 dBFS），配合"录 3 秒环境采样自动校准"按钮
- **触发模式可切换**：
  - 长按模式（默认 = 当前实现）：按住 hotkey 录音，松开停止
  - 切换模式：点一下 hotkey 开始，再点一下结束（适合长段口述、避免按累）
- 剪贴板恢复（粘贴完恢复原内容）
- 简单历史记录（内存中保留最近 N 条）

### Phase 3：macOS 适配（`macos` 分支进行中）
平台差异都用 `#[cfg(target_os = "macos")]` 收口，与 Windows 共用一份源码。

已做：
- 粘贴键 ⌘V（`paste.rs`，enigo `Key::Meta`）
- 快捷键 macOS 化：默认右 Option(⌥)，新增 RightCommand / Fn 等键名（`hotkey.rs`）
- 原生构建：`.cargo/config.toml` 去掉强制 Windows target，`cargo build` 默认出 Mach-O
- 权限缺失的兜底：rdev 监听挂掉时打印中文权限提示并干净退出，不再空转刷屏
- 麦克风权限引导文档（README）

未做：
- 麦克风 / 输入监控 / 辅助功能权限只能靠文档引导，无法代码内授权
- 签名 / 公证 / `.app` 打包（需 Apple 开发者证书）
- 完整录音→转写→粘贴链路的真机验证（依赖上述权限 + `GROQ_API_KEY`）

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
