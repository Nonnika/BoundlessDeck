# BoundlessDeck-RS

[English](../README.md) · 简体中文

无限画布工作台，界面由 [GPUI](https://www.gpui.rs/) 原生绘制，是 BoundlessDeck（Go 版）的 Rust 重写。一个文件夹就是一张 deck：画布保存在 `<文件夹>/.boundlessdeck/deck.json`，格式与 Go 版相同，两版可以互开。

## 理念

### 1. 一切皆 block

便签、Markdown 笔记、文档、终端、网页、图表、块堆，都是 **block**：同一个模型，有位置、大小和生命周期。它们以同样的方式移动、缩放、堆叠、选择、撤销和保存，也由同一张字段表描述给 agent。所以画布上能放的东西，agent 都能读、能改。

### 2. agent 驱动画布

打开 deck 就会启动一个 MCP 端点。agent 不只是在对话里回答，而是直接在画布上工作：新建和修改 block、排版、对齐和均分、选中，再把你的视野移过去给你看它做了什么。agent 的每一次改动都和你的操作走同一个撤销栈，⌘Z 就能撤回。

### 3. 环境即上下文

你不用解释自己在哪。deck 文件夹就是工作区。在终端 block 里启动的 shell 知道自己是哪个 block（`BOUNDLESSDECK_TERMINAL`）、属于哪张 deck（`BOUNDLESSDECK_DECK`），从这个终端里启动的 agent 也知道。你选中了什么、正在操作哪个终端、什么挨着什么，agent 都能拿到（`read_selection`、`current_terminal`、`deck_info`）。你怎么摆放画布，就是你给的提示。

## 开始使用

```sh
cargo run                    # 主页
cargo run -- <文件夹>        # 直接把该文件夹作为 deck 打开
cargo test
cargo bundle --release       # 打包 .app（需 cargo-bundle）
```

试运行时把 `BOUNDLESSDECK_DATA_DIR` 与 `BOUNDLESSDECK_ACTIVE_FILE` 指到临时目录，不碰真实数据。

## 画布上有什么

- **QuickLook** 便签（会到期消失）、**笔记**（Markdown 文件，实时渲染，勾选框可点击）、**文件**预览（文本、Markdown、图片、PDF、Word、视频）
- **终端**（alacritty_terminal，支持输入法）、**浏览器**（WKWebView，多标签页）、**图表**、**块堆**
- 直接画在画布上的**文字**与**图形**（矩形、椭圆、菱形、三角），在右侧面板调整颜色、字体（面板旁可搜索的列表，任何已安装字体都在其中）、字号、圆角（可四角分别设置）与对齐
- 平移、捏合与 ⌘ 滚轮缩放、框选、撤销重做；每个缩放档位都重新排版文字，放多大都清晰
- 首次启动的欢迎引导介绍主要功能（可在设置里重新打开），以及可选的强调色

## 给 agent 用

打开 deck 即启动 MCP 端点，`boundlessdeck mcp-stdio` 是它的 stdio 桥。19 个工具可读写、排版 block。在设置（⌘,）里可以为 Claude Code、Claude 桌面版、Cursor、VS Code、Codex、OpenCode、oh-my-pi 一键安装全局配置。

## Windows

Windows 版不提供安装包，请自行编译：安装 [Rust](https://rustup.rs/)（1.85 及以上）后在仓库目录运行 `cargo build --release`，产物在 `target\release\boundlessdeck.exe`。PDF 预览、捏合缩放、网页快照、模糊等依赖 macOS 系统接口的功能在 Windows 上不可用或有所降级。

## 尚未做到

- 终端不能用鼠标选择文字，不支持鼠标上报
- 文本编辑器没有自己的撤销

开发说明见 [`AGENTS.md`](../AGENTS.md)。以 MIT 许可证发布。
