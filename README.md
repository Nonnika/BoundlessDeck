# BoundlessDeck-RS

无限画布工作台，BoundlessDeck（Go 版）的 Rust 重写，界面由 GPUI 原生绘制。一个文件夹就是一张 deck，画布存在 `<文件夹>/.boundlessdeck/deck.json`，与 Go 版互通。

```sh
cargo run                    # 主页
cargo run -- <文件夹>        # 直接打开该文件夹
cargo test
cargo bundle --release       # 打包 .app（需 cargo-bundle）
```

试运行时把 `BOUNDLESSDECK_DATA_DIR` 与 `BOUNDLESSDECK_ACTIVE_FILE` 指到临时目录，不碰真实数据。

## 画布上有什么

- **QuickLook** 便签（会到期消失）、**笔记**（Markdown 文件）、**文件**预览（文本、Markdown、图片、PDF、Word、视频）
- **终端**（alacritty_terminal，支持输入法）、**浏览器**（WKWebView，多标签页）、**图表**、**块堆**
- 平移、捏合与 ⌘ 滚轮缩放、框选、撤销重做；缩放后内容渐变变清晰

## 给 agent 用

打开 deck 即启动 MCP 端点，`boundlessdeck mcp-stdio` 是 stdio 桥。18 个工具可读写、排版 block；`current_terminal` 告诉 agent 它运行在哪个终端里。设置窗口（⌘,）可为 Claude Code、Claude 桌面版、Cursor、VS Code、Codex、OpenCode、oh-my-pi 一键安装全局配置。

## 尚未做到

- 终端不能用鼠标选择文字，不支持鼠标上报
- 文本编辑器没有自己的撤销
- GPUI 没有模糊滤镜，主页与过渡里的“模糊”都是淡入淡出近似

开发说明见 `AGENTS.md`。
