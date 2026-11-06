# AGENTS.md

工程记忆：给在本仓库工作的 agent 看的操作手册。产品需求见 [docs/agent-workbench.md](docs/agent-workbench.md)，技术规格见 [docs/requirements.md](docs/requirements.md)。

## 这是什么

BoundlessDeck：原生 UI（MyGo 自己绘制，**不是 WebView**）的无限画布工作台，面向创作者与学生，最终形态是 agent 驱动。Go 1.27 + `github.com/egoist/mygo` v0.7.4。没有 JS、没有 npm、没有 IPC 绑定层。

**术语硬约定**：deck 里的所有元素都叫 **block**（笔记、文件、终端……都是 block 的不同 `kind`）。代码与数据里不出现 `node` / `card`，JSON 字段是 `"blocks"`。

## 命令

```sh
go tool mygo dev      # 开发模式：编译 → 启动 → 改 .go 自动重启
go tool mygo build    # 打包 build/<os>-<arch>/
go test ./...         # 无窗口渲染测试（ui.NewTester）
go test -race ./...
go tool mygo vet .    # 检查原生 UI 的构建期生命周期（必跑）
DECK_SNAPSHOT=1 go test -run TestSnapshot .   # 重画 docs/canvas-snapshot.png
```

`mygo` 通过 `go.mod` 的 `tool` 指令提供，不要去找全局安装的 `mygo`。

## 架构地图

| 文件 | 职责 |
|---|---|
| `main.go` | 应用入口：`WindowOptions{Content: ui.View(d.view)}`、`StateKey`、`OnFileDrop`、关窗保存并挂断终端 |
| `deck.go` | 数据与状态：`block`、`camera`、增删/撤销重做、`freeSpot`、拖放、`deck.json` 读写 |
| `canvas.go` | 一帧的画布：网格、输入仲裁（平移/缩放/框选/拖动）、工具栏、状态栏、文件对话框 |
| `blocks.go` | 单个 block 的渲染：标题栏、note、file（editor / bitmap）、terminal（懒启动）、stack（钱包卡片） |
| `chart.go` | 图表 block：`Text` 里的表格（逗号/Tab/Markdown 表格）画成柱状/条形/折线/面积/饼图（`block.Chart`），标题栏切换类型、双击或表格按钮编辑数据 |
| `stack.go` | 块堆 block：`Items` 收起的 block ID、点击翻卡、空格长久预览、取出；被收进的 block 仍在 `d.blocks` 里，只是不画在画布上 |
| `memo.go` | 笔记 block：deck 根目录里的 Markdown 文件，渲染、编辑、自动写盘、外部改动回读 |
| `canvas_test.go` | 15 个 `ui.NewTester` 测试：相机数学、输入、加删、撤销、LOD、持久化 |
| `mcpschema.go` | block 字段的唯一真相：每种 kind 的可写/只读字段、默认值、约束，生成工具 schema 与 `.boundlessdeck/block-schema.json` |
| `mcpdeck.go` | MCP 的读写语义：字段校验、原子 patch、创建/删除/移动/选择、对外 JSON 形状 |
| `mcplayout.go` | 排版：对齐、等距分布、行/列/网格排列；只改 x/y，经 `applyPlacements` 成为一次可撤销的移动 |
| `mcpserver.go` | 工作区的 MCP 端点：Unix socket 上的 JSON-RPC（tools/resources）、token、UI 线程调度 |
| `mcpworkspace.go` | 工作区级配置：`.boundlessdeck/mcp.json`、`.mcp.json`、`.cursor/mcp.json`、`.vscode/mcp.json` 的写入/合并/加载 |
| `mcpbridge.go` | stdio ↔ socket 桥：`boundlessdeck mcp-stdio --workspace DIR` |

## MCP（画布对外接口）

外部 agent（Claude Code、Cursor、VS Code）通过工作区自己的 MCP 读写这张画布，和窗口里的人是同一条路径。

- **schema 只有一份**：`mcpschema.go` 的 `mcpFields`。给 `block` 加字段就必须在表里登记类型、是否可写、属于哪些 kind；`TestMCPSchemaCoversEveryBlockField` 用反射盯着，漏了就红。不要手写第二份字段列表。
- **写操作只有一条路**：`mcpPatchBlock` → `addBlock`/`removeBlock`，与 UI 共用撤销栈、`freeSpot`、选择集和保存。批量写先 `mcpValidatePatch` 干跑，全部合法才落笔。
- **移动与排版可撤销**：`mcplayout.go` 先算目标位置、再经 `applyPlacements` 一次性写入，产生一条 `changeMoves`（记 before/after），⌘Z/`undo` 一档退回整次排版。不要在排版函数里直接改 `b.X/b.Y`，也不要绕过 `applyPlacements`——否则会丢撤销记录。
- **排版工具**：`align_blocks`（`align` + `to`=bounds/first/last/block ID，可选 `spacing` 排成列）、`distribute_blocks`（`axis`、`mode`=gaps/centers、`allowOverlap`）、`arrange_blocks`（`layout`=row/column/grid、`columns`、`gapX/gapY`、`originX/originY`、`order`=ids/position）。省略 `ids` 用选择集。
- **线程**：HTTP handler 在 goroutine 里，碰 deck 一律走 `mcpServer.onUI`（`win.Update`，15s 超时）。`win == nil` 的测试里直接执行。
- **工作区即端点**：`openFolder` 里 `d.startMCP()` 起 socket（`<root>/.boundlessdeck/mcp.sock`；路径过长或平台没有 Unix socket 时退到 `/tmp` 或 127.0.0.1 端口）并把项目级配置写进工作区；`closeDeck` 里 `d.stopMCP()`。配置是**合并**：用户已有的 server、key 原样保留；坏 JSON 先备份成 `.bak`。
- **保存**：MCP 写操作调 `requestSave`：UI 线程 marshal，goroutine 写盘，`saveSeq` 保证只有最新快照落地；无窗口时同步写。
- **桥**：项目配置只写 `command + args`，不写端口；桥启动时读 `.boundlessdeck/mcp.json` 拿 socket 与 token。手动排障：`./boundlessdeck mcp-stdio --workspace DIR --check`。
- 状态栏右下角显示 `MCP · N 个工具`，tooltip 里有 socket、描述文件与项目 server；启动失败显示 `MCP 未启动`。

## 帧模型（改代码前必读）

视图是 `func (d *deck) view(c *ui.Context)`，**每帧重建整棵树**。因此：

- 视图里只读状态、只建元素。**慢 I/O 一律 goroutine，结果用 `win.Update(func(){...})` 回主线程**（`loadText` / `loadImage` / `startTerminal` 是范本）。
- 元素与 `ui.Element` 值只在本次 build 有效，**不能存进 `d`**。要跨帧的东西放 `block.rt`（`*runtime`）或 `ui.Handle`。
- 状态变更要么发生在 `OnClick`（延迟执行）里，要么发生在视图构建中但只碰副本：`×` 按钮在构建中直接 `removeBlock`，能这么写是因为遍历的是 `visible()` 返回的副本；改成遍历 `d.blocks` 本体就会边遍历边删。

## 三条平台约束（已核实，别绕开）

1. **不用 transform 做缩放。** `ui.WebView` 在变换盒子里只覆盖矩形、内容不缩放（`mygo/ui/webview.go:64`），且祖先变换会保留原布局空间。画布用**真缩放**：世界坐标乘 `cam.S` 后重新布局（`blockView` 里 `Left/Top/Size` 都乘过）。放大后文字清晰也是这个原因。
2. **一次按下只交给一个 handler。** `rt.handler(chain)` 只把 press 交给链上最内层 handler（`mygo/ui/input.go:349`）；滚轮/捏合才按链顺序问到有人消费。所以 block 自己有 `HandleInput`（选中/拖标题栏），而编辑器、终端更内层，点进去照常落光标；block 把滚轮与捏合**换算回窗口坐标转发给画布**（`onBlockInput` 的第一个 case）。
3. **必须 LOD。** `scale < lodScale(0.3)` 时 `blockView` 只画框和标题，不建编辑器/终端/位图。官方基准：1000 静态 label ≈ 0.4ms/帧，但一个 `editor.View` 或 `terminal.View` 远不止这个量级。

补充：`mygo.Dialog.Open` 必须从 goroutine 调（`pickFile`），结果用 `win.Update` 回来；在主线程直接调会卡住事件循环。

## 不改的约定

- **block 只能通过 `addBlock` 进入 deck，只能通过 `detach` 离开。** 撤销栈依赖这一点，否则会出现"某条路径加的 block 撤不掉"。
- `detach` 会挂断终端（`hangUp`）；撤销回来时 `terminalBody` 重新启动一个。删掉的 shell 不留僵尸进程。
- 新 block 一律经 `freeSpot` 落位，不精确叠在旧 block 上。
- **块堆只改 `Items`，不搬 block。** 收进/取出走 `stackAdd` / `stackRemove` / `setStackItems`，经 `changeStack` 记 before/after 成为一次可撤销的变更；被收进的 block 仍由 `addBlock`/`detach` 管理，`visible()` 与 `selectIn` 跳过它们。不要从 `d.blocks` 里删掉堆里的 block，否则撤销、终端恢复和 MCP 的 ID 都会断。
- 覆盖层（如块堆的长预览）**不要在容器上挂 `HandleInput`**：它会先拿到按下，子按钮就点不动。背景层负责点外部关闭，子控件各自挂 handler（`stackButton`）。
- 链接外部文件的路径经 `linkPath` 相对化到工作区根；工作区因此可以整体搬移。
- `runtime`（编辑器、位图、终端）**永不进 JSON**，重启后重建。
- 写盘走原子写：临时文件 + rename，失败静默（后续要加错误提示）。

## 数据格式

`App.Path(PathUserData)/deck.json`：

```jsonc
{ "version": 1, "root": "<工作区根>", "camX": 0, "camY": 0, "camS": 1,
  "blocks": [ { "id": "b1a2c3", "kind": "note|memo|file|terminal|browser|stack", "title": "", "x": 0, "y": 0, "w": 360, "h": 260,
                "path": "相对工作区根的文件路径", "text": "笔记正文", "items": ["b4d5e6"] } ] }
```

`items` 只属于 `stack`：堆里的 block ID，按卡片顺序；被收进的 block 仍在 `blocks` 里。`version` 仍是 1，新字段是可选的增量。

改结构必须升 `version` 并在 `load()` 里加迁移。

## 测试约定

- 用 `ui.NewTester(d.view, w, h)` 驱动：`Press/Move/Release/ClickAt/Wheel/Pinch/Key/Image`。注意 `Tester.Wheel` 不带修饰键（引擎会用事件里的 `Mods` 覆盖），测 `⌘+滚轮` 要直接构造 `ui.InputEvent` 调 `onCanvasInput`。
- 新增交互 = 新增一个断言真实状态的测试（相机数值、`d.blocks`、`d.sel`），不要只测"没崩"。
- 视觉改动跑一次快照并肉眼比对 `docs/canvas-snapshot.png`。
- 提交前：`gofmt -l .` 无输出、`go test ./...`、`go tool mygo vet .`。

## 现状与下一步

已完成：native 骨架、相机与输入、block 增删 + 撤销重做 + 拖放落位、file（文本/图片/md/pdf/word 预览）、terminal、browser、memo、块堆（收 block、翻卡、空格长久预览）、LOD、持久化、工作区 MCP（见上）。
进行中（M1）：markdown 预览、失效态、`mygo.WatchFiles` 刷新、大文件分段、外部打开。
之后（M2）：代码卡可编辑 + 写盘冲突 + `⌘↵` 运行器。
未做（M3+）：终端与画布快捷键仲裁、block resize、小地图、搜索、i18n、打包签名。