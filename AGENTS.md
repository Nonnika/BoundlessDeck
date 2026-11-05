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
| `blocks.go` | 单个 block 的渲染：标题栏、note、file（editor / bitmap）、terminal（懒启动） |
| `canvas_test.go` | 15 个 `ui.NewTester` 测试：相机数学、输入、加删、撤销、LOD、持久化 |

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
- 链接外部文件的路径经 `linkPath` 相对化到工作区根；工作区因此可以整体搬移。
- `runtime`（编辑器、位图、终端）**永不进 JSON**，重启后重建。
- 写盘走原子写：临时文件 + rename，失败静默（后续要加错误提示）。

## 数据格式

`App.Path(PathUserData)/deck.json`：

```jsonc
{ "version": 1, "root": "<工作区根>", "camX": 0, "camY": 0, "camS": 1,
  "blocks": [ { "id": "b1a2c3", "kind": "note|file|terminal", "title": "", "x": 0, "y": 0, "w": 360, "h": 260,
                "path": "相对工作区根的文件路径", "text": "笔记正文" } ] }
```

改结构必须升 `version` 并在 `load()` 里加迁移。

## 测试约定

- 用 `ui.NewTester(d.view, w, h)` 驱动：`Press/Move/Release/ClickAt/Wheel/Pinch/Key/Image`。注意 `Tester.Wheel` 不带修饰键（引擎会用事件里的 `Mods` 覆盖），测 `⌘+滚轮` 要直接构造 `ui.InputEvent` 调 `onCanvasInput`。
- 新增交互 = 新增一个断言真实状态的测试（相机数值、`d.blocks`、`d.sel`），不要只测"没崩"。
- 视觉改动跑一次快照并肉眼比对 `docs/canvas-snapshot.png`。
- 提交前：`gofmt -l .` 无输出、`go test ./...`、`go tool mygo vet .`。

## 现状与下一步

已完成：native 骨架、相机与输入、block 增删 + 撤销重做 + 拖放落位、file（文本/图片）与 terminal 两种 block、LOD、持久化。
进行中（M1）：markdown 预览、失效态、`mygo.WatchFiles` 刷新、大文件分段、外部打开。
之后（M2）：代码卡可编辑 + 写盘冲突 + `⌘↵` 运行器。
未做（M3+）：终端与画布快捷键仲裁、block resize、小地图、搜索、i18n、打包签名。