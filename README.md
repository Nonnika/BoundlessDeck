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

## MCP（给 agent 的画布接口）

打开一个 deck，它就是一个 MCP 工作区：`.boundlessdeck/mcp.sock` 上暴露这张画布的 block（标题、位置、大小、正文、文件路径、PDF 页、网址……可写项见 `block_schema`），并把同一个 server 自动合并进 `.mcp.json`、`.cursor/mcp.json`、`.vscode/mcp.json`。用 Claude Code / Cursor 在这个文件夹里起步即可，不用手动配。

```sh
./boundlessdeck mcp-stdio --workspace . --check   # 看端点是否在（App 开着这个 deck 时）
```

工具：`deck_info`、`block_schema`、`list_blocks`、`get_block`、`read_selection`、`create_block`、`update_block`、`update_blocks`、`delete_blocks`、`move_blocks`、`select_blocks`、`undo`、`redo`、`save_deck`，以及排版三件套：

| 工具 | 作用 |
|---|---|
| `align_blocks` | 对齐左/右/上/下/水平居中/垂直居中；`to` 选外框、首个、末个或某个 block；给 `spacing` 顺手排成一列/一行 |
| `distribute_blocks` | 三个以上 block 沿 x/y 等间隙或等中心距排开，首尾不动；挤不下要显式 `allowOverlap` |
| `arrange_blocks` | 排成一行、一列或网格（列宽行高取最大，可设间距、列数、起点、按位置排序） |

省略 `ids` 时排版工具用当前选择集。新增/删除、`move_blocks` 与排版都进撤销栈，并立即保存。

块堆（`kind: "stack"`）通过 `create_block` / `update_block` 的 `items` 字段收放 block：数组是堆里的 block ID，按卡片顺序，不能装自己或另一个块堆；被收进的 block 仍在 `list_blocks` 里，只是不画在画布上。

## 术语

deck 里的所有元素统一叫 **block**（QuickLook 便利贴、文件、终端……都是 block 的不同 kind；QuickLook 在代码与数据里的 kind 仍是 `note`）。代码里没有 `node`、`card`，只有 `block`；数据文件里是 `"blocks"`。

## 操作

| 操作 | 手势 |
|---|---|
| 平移 | 拖空白 / 中键 / 右键拖；滚轮与触控板（在未聚焦的 block 上也能平移） |
| 滚动终端 / 文件内容 | 先点击它聚焦，再滚动；点空白处取消聚焦 |
| 缩放 | `⌘`+滚轮、双指捏合、`⌘+` `⌘-` `⌘0` |
| 新建 | 右上角 `QuickLook` / `文件…` / `终端` / `块堆`；空白双击建 QuickLook；从访达拖入文件 |
| 块堆 | 拖 block 的胶带或标题栏到堆上收进去；点击翻下一张；`空格` 打开长久预览（`空格`/`Esc` 关闭，`←` `→` 翻卡），预览里的「取出」放回画布 |
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