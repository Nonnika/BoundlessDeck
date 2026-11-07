# BoundlessDeck-RS

BoundlessDeck（Go + MyGo 版，见上级目录）的 Rust 重写，界面用 GPUI（Zed 的 GPU UI 框架）原生绘制。

```sh
cargo run                      # 主页
cargo run -- <文件夹>          # 直接打开该文件夹的 deck
cargo test                     # 模型、持久化、编辑器外的纯逻辑测试
```

本机没有 Xcode 的 `metal` 编译器，所以 gpui 开启了 `runtime_shaders` 特性（运行时编译着色器）。

## 与 Go 版的兼容

`.boundlessdeck/deck.json` 格式与 Go 版一致（字段名、省略空值、终端历史的 base64、时间格式），两版可以打开同一个文件夹。未实现类型的 block 数据原样保留。

## 第一阶段已完成

- 画布：平移（双指/右键/中键拖）、⌘+滚轮缩放并吸附到缩放档位、⌘0 / ⌘= / ⌘-、框选、Shift+点击、⌘D、⌘Z / ⇧⌘Z、双击空白新建 QuickLook、顶部空白拖窗口
- block：胶带拖动、标题栏拖动、点标题改名、右下角等比缩放、LOD（< 30% 只画框和标题）、拖进块堆
- QuickLook（便签、Markdown、到期淡出与消失、剩余时间菜单）、笔记（Markdown 文件、自动保存、外部修改回读、改名即改文件名）、文件（文本带行号、Markdown、图片）、终端（alacritty_terminal；关闭时保存历史与工作目录，再打开时恢复）
- 主页：最近的 deck、打开文件夹、拖入文件夹、移除记录；菜单栏与快捷键

## 尚未移植

浏览器 block、PDF / Word 预览（目前提供“用默认应用打开”）、块堆的展开/拔出/长预览（目前只列出卡片）、图表的完整绘制（条形/面积图、坐标刻度、图例、提示）、MCP 端点与设置窗口、主页 3D 场景、键盘快捷键窗口、触控板捏合缩放（gpui 0.2.2 没有捏合事件，用 ⌘+滚轮）。
