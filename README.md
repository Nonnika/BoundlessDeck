# BoundlessDeck

无限画布工作台，原生 UI（MyGo 直接绘制，非 WebView）。

三条主线需求：本地文件超链接与预览、代码嵌入、终端。需求与架构见 [docs/requirements.md](docs/requirements.md)。

## 运行

```sh
go tool mygo dev      # 开发模式，改 .go 自动重启
go tool mygo build    # 打包到 build/<os>-<arch>/
```

macOS 13+（终端用 libghostty-vt）。

## 打开 deck

启动后是主页：点「打开文件夹…」（`⌘O`）或把文件夹拖进窗口，这个文件夹就是一张 deck。画布和 block 保存在文件夹里的 `.boundlessdeck/deck.json`，终端从这个文件夹启动，文件链接存成相对它的路径。主页列出最近打开的文件夹；deck 里点左上角的文件夹名会保存并回到主页。

## 术语

deck 里的所有元素统一叫 **block**（QuickLook 便利贴、文件、终端……都是 block 的不同 kind；QuickLook 在代码与数据里的 kind 仍是 `note`）。代码里没有 `node`、`card`，只有 `block`；数据文件里是 `"blocks"`。

## 操作

| 操作 | 手势 |
|---|---|
| 平移 | 拖空白 / 中键 / 右键拖；滚轮与触控板（在未聚焦的 block 上也能平移） |
| 滚动终端 / 文件内容 | 先点击它聚焦，再滚动；点空白处取消聚焦 |
| 缩放 | `⌘`+滚轮、双指捏合、`⌘+` `⌘-` `⌘0` |
| 新建 | 右上角 `QuickLook` / `文件…` / `终端`；空白双击建 QuickLook；从访达拖入文件 |
| 删除 | 选中后 `⌘D`（Windows 为 `Ctrl+D`），或工具栏 `删除` |
| 撤销 / 重做 | `⌘Z` / `⌘⇧Z`（加删 block 都可撤销） |
| 移动卡片 | 拖顶部伸出的胶带或标题栏（点击胶带会选中 block） |
| 调整大小 | 拖右下角（QuickLook 是折角，其他 block 是斜纹把手），等比缩放 |
| 编辑 QuickLook | 单击标题改标题、单击正文改内容（Markdown）；`Esc`、`⌘↵` 或点别处完成，标题按回车完成 |
| 选择 | 框选、Shift+点击加选/取消；单击卡片直接操作，不会选中；空白单击清空 |

## 开发

```sh
go test ./...                        # 画布与持久化的单测（无窗口）
go test -race ./...
go tool mygo vet .                   # 检查原生 UI 的构建期生命周期问题
DECK_SNAPSHOT=1 go test -run TestSnapshot .   # 重画 docs/canvas-snapshot.png
```