# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

完整的操作手册在 `AGENTS.md`（架构、帧模型、输入仲裁、缩放、撤销、保存、MCP 数据流、约定与验证流程），以下导入它，不在此重复：

@AGENTS.md

## 补充（AGENTS.md 未覆盖）

- **常用命令速查**：`cargo run -- /tmp/test-deck`（直接打开 deck）、`cargo test`、`cargo test model::tests::<name>`（单个测试）、`cargo clippy`（不新增警告）。不要运行整仓 `cargo fmt`。
- **`src/stage.rs`**：主页与 deck 之间切换时的整窗模糊过渡。GPUI 不支持模糊，所以用 Core Image 滤镜加在窗口 view 上（会连同 WKWebView 一起模糊）。时长与模糊量与 Go 版一致。
- **`src/webfocus.rs`**：WKWebView 的键盘与滚轮仲裁。只有用户点进过的网页才拿键盘；其他网页若自行成为 first responder，每帧及每次按键前把焦点还给 GPUI view。未点进的网页上的双指滚动平移画布；落在 resize 手柄上的按下事件交给 deck。改 `deck/browser.rs` 的输入行为时要同时看这里。
- **提交信息**：type 与 scope 用英文小写，主题和正文用简体中文，无句号，例如 `feat(canvas): 支持触控板捏合缩放`。
