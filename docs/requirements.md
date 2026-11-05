# BoundlessDeck 需求与架构 v0.2（native）

> 结论：画布走 **原生 UI（`ui` 包）**，整个应用是纯 Go，无 WebView 页面、无 npm、无 IPC 绑定层。
> 本文基于 mygo **v0.7.4** 的实际源码核对写成（`~/go/pkg/mod/github.com/egoist/mygo@v0.7.4`），并附已落地的骨架状态。

## 0. 为什么 native 方案对这三条需求反而更划算

| 需求 | WebView 方案（v0.1） | native 方案（本文） |
|---|---|---|
| 终端 | xterm.js + 自写 PTY + base64 流 + 背压 | `plugins/terminal`：Ghostty 的 libghostty-vt，`terminal.New` + `terminal.View`，**PTY 层官方做完**（`plugins/terminal/view.go:34`） |
| 代码 | CodeMirror + 文件读写走绑定 | `ui/editor`（Zed 风格，虚拟化到行，自带语言/折叠/多光标/诊断），文件直接 `os` 读写 |
| 文件预览 | 自定义 scheme + 沙箱 iframe + CSP | Go 直接读 → `editor.View` / `ui.Image` / `ui/markdown`，**没有可信 origin 越权问题** |
| 依赖 | bun + vite + 4 个 npm 包 + mygo-runtime | 一个 `go build`，`go tool mygo build` 出安装包 |

代价也非常明确，见第 2 节的三条硬约束。

## 1. 已核对的平台能力（v0.7.4）

| 能力 | API | 依据 |
|---|---|---|
| 原生视图 | `mygo.WindowOptions{Content: ui.View(fn)}` | `window.go:201` |
| 绝对定位 | `Element.Absolute().Left(x).Top(y).Size(w,h)`，不入父布局 | `ui/api_gen.go:2717`、`layout.go:1178` |
| 裁剪 | `.Clip()` / `.ClipX()` / `.ClipY()` | `ui/element.go:1034` |
| 原始输入 | `.HandleInput(func(ev ui.InputEvent) bool)`，`ev.X/Y` 已换算成元素局部坐标 | `ui/handler.go:58`、`input.go:144` |
| 捏合缩放 | `InputZoom{Scale}`；Windows 把 Ctrl+滚轮也报成缩放 | `ui/handler.go:48` |
| 绘制 | `.Draw(func(p *ui.Painter, r ui.Rect))`：矩形/路径/文本/位图/SVG | `docs/ui/drawing.md` |
| 代码编辑器 | `editor.View(c, &editor.State{Buffer: editor.NewBuffer(text), Font: ui.Font{Size: 12}})` | `ui/editor/build.go:22`、`state.go:13` |
| 终端 | `terminal.New(terminal.Options{Dir, OnTitle, OnExit})` + `terminal.View(c, t)` | `plugins/terminal/view.go:34` |
| 图片 | `ui.DecodeBitmap` / `ui.DecodeImage` + `ui.Image(c, src).Fit(ui.Contain)` | `ui/widgets.go:536` |
| Markdown | `markdown.View(c, src, *Options)` | `ui/markdown/view.go:98` |
| 文件对话框 | `mygo.Dialog.Open（Multiple/Directory/Filters）`；必须从 goroutine 调，结果用 `win.Update` 回主线程 | `dialog.go:128`、`content.go:172` |
| 文件监听 | `mygo.WatchFiles(ctx, path, opts)`（原生不需要 watch 插件） | `watch.go:168` |
| 外部打开 | `mygo.Shell.OpenPath` / `OpenExternal` / `ShowItemInFolder` | `modules.go:34` |
| 窗口状态 | `StateKey` 记忆位置尺寸 | `window.go` |
| 测试 | `ui.NewTester(view, w, h)`：`Press/Move/Release/ClickAt/Wheel/Pinch/Key/Image` | `ui/headless.go`、`api_gen.go:1437` |
| 端到端 | `plugins/automation`（socket 驱动运行中的应用，截图/输入） | `docs/plugins/automation.md` |

## 2. 三条硬约束（决定了画布怎么实现）

**① `ui.WebView` 不能缩放。** 源码注释写死："Native web views stay upright, covering a transformed box's bounds"（`ui/webview.go:64`）。所以任何祖先 `Scale/Translate` 变换，只改变它覆盖的矩形，不改变它的内容比例。
→ 画布**不用 transform 缩放**，改成**真缩放**：把 `n.X/n.Y/n.W/n.H` 乘上相机比例后重新布局。副作用正好是我们想要的：文本、编辑器和终端都按最终字号布局，放大不糊。

**② 立即模式：每帧重建整棵树。** 官方基准 1000 个静态 label = 0.397ms/帧、0 分配（`docs/ui/performance.md`）。
→ 画布可以每帧重建，但**重的东西不能每帧建**：`scale < 0.3` 时只画卡片框和标题（LOD），视口外 64px 以外的节点不构建；静态底色用 `ui.Canvas`/`Cache` 缓存。

**③ 一次按下只有一个 handler。** `rt.handler(chain)` 只把按下事件交给命中最内层的 handler（`ui/input.go:349`）；滚轮/缩放则会按链顺序问每一个 handler，直到有人消费（`input.go:500`）。
→ 卡片自己接管"按下"（选中 / 标题栏拖动），终端和编辑器是更内层 handler，所以点进文本照常落光标；卡片把滚轮和捏合**转发回画布**（换回窗口坐标），保证在卡片上滚轮也是在移动画布。

## 3. 画布核心需求

| 编号 | 需求 | 优先级 | 状态 |
|---|---|---|---|
| C-1 | 平移：左键拖空白（框选优先）/ 中键或右键拖 / 滚轮与触控板 | P0 | ✅ |
| C-2 | 缩放：`⌘/Ctrl+滚轮`、捏合、`⌘+ / ⌘- / ⌘0`，范围 0.08–4，指针为锚点 | P0 | ✅ |
| C-3 | block：绝对世界坐标、`note / file / terminal`（后续 `code`）、标题栏拖动、`⌘D`/`Ctrl+D` 删除选中 | P0 | ✅ |
| C-4 | 选择：点选、Shift 加选、框选、空白单击清空 | P0 | ✅ |
| C-5 | LOD：`scale < 0.3` 只画框与标题，不建编辑器/终端/位图 | P0 | ✅ |
| C-6 | 持久化：`deck.json` 原子写（tmp+rename），含相机与节点，关窗保存 | P0 | ✅ |
| C-7 | `⌘0` 全览所有卡片（fit） | P1 | ✅ |
| C-8 | 缩放锚点数学：`zoomAt` 保持指针下的世界点不动 | P0 | ✅（有单测） |
| C-9 | 撤销/重做：命令栈，拖动/缩放合并为一条 | P0 | 部分（加删 block 已可撤销） |
| C-10 | 缩放手柄 resize、对齐吸附、小地图、`⌘F` 搜索飞行 | P1/P2 | ⏳ |
| C-11 | 世界坐标用 `float32`：±1e5 时精度约 0.01 DIP，超出范围要限制（防漂移） | P1 | 待定 |
| C-12 | 大图性能：1000 卡片、视口内 80 张、含 8 个终端 → ≥ 55fps | P0 | 待压测 |

## 4. 需求 1：本地文件超链接与预览

### 4.1 语义

`file` 卡片 = 路径 + 可选定位（`line:col` / 标题锚点）+ 预览体。**预览在 Go 里直接读文件**，没有 origin、没有协议、没有 iframe。

| 编号 | 需求 | 优先级 |
|---|---|---|
| F1-1 | 建立链接：`文件…` 按钮（`mygo.Dialog.Open`）、拖放（`FileDropEvent`）、粘贴路径 | P0（按钮已通） |
| F1-2 | 路径规则：绝对路径原样保存；工作区内保存为相对工作区根，工作区可整体搬移 | ✅ |
| F1-3 | 去重：同一文件已被链接时，跳到已有卡片而不是新建 | ✅ |
| F1-4 | 失效态：不存在/无权限/被删 → 卡片内明确错误文案 + "重新定位" | 部分（错误已显示） |
| F1-5 | 监听刷新：`mygo.WatchFiles` 变化后 300ms 重读，保留编辑器滚动位置 | P1 |
| F1-6 | 大文件：>4 MiB 只读头部并提示；二进制检测后给"十六进制/外部打开" | P1 |
| F1-7 | 编码：UTF-8/BOM/UTF-16 探测，非法字节替换 | P1 |
| F1-8 | `⌘⇧O` 用系统程序打开，`⌘⇧R` 在 Finder 中显示 | P1 |

### 4.2 预览矩阵（native 版）

| 类型 | 实现 | 备注 |
|---|---|---|
| 文本 / 代码 | `editor.View`（只读，`LineNumbersOn`，字号随缩放） | 已实现 |
| 图片 / SVG | `ui.DecodeImage` → `ui.Image(c, src).Fit(ui.Contain)` | 已实现 |
| Markdown | `ui/markdown.View` | P1 |
| PDF | **没有原生渲染器**：`ui` 只有 `pdf_text` 文本抽取；用单个 `ui.WebView` 预览（每窗口最多 1 个）或交系统打开 | P1 |
| 视频 | `ui.Video` 需要应用自备 `VideoDecoder`（`ui/video.go:107`）→ v1 交系统打开 | P2 |
| 音频 | 同上 | P2 |
| HTML | 单个 `ui.WebView` + `PageOptions{Untrusted: true}`，或直接把 HTML 交系统浏览器 | P2 |
| Office / 压缩 / 未知 | 元信息卡 + 系统打开 | P1 |

> 注意：`ui.WebView` 是**独立 web 进程**，每张卡片一个会吃掉数百 MB 且不可缩放（约束①）。因此 HTML/PDF 预览定为"**单实例、固定缩放的预览面板**"，不进无限画布的缩放体系。

## 5. 需求 2：代码嵌入

| 编号 | 需求 | 优先级 |
|---|---|---|
| F2-1 | 代码卡：`editor.View` + `editor.State`（语言、折叠、多光标、搜索、软换行、行号） | P0 |
| F2-2 | 两种源：`inline`（片段存 deck.json）与 `file`（绑定路径，脏标记） | P0 |
| F2-3 | 保存：`⌘S` 原子写，写前比对 mtime+哈希；外部已改 → 覆盖/重载/看差异（`editor.DiffView`） | P0 |
| F2-4 | 只读默认开，`⌘E` 切换 | P0 |
| F2-5 | 运行：`⌘↵`，`exec.CommandContext` + 独立进程组（`Setpgid` / Windows Job Object），30s 超时，`⌘.` 终止 | P1 |
| F2-6 | 输出：卡片底部输出区，stdout/stderr 分流；结果随卡片持久化（最近一次） | P1 |
| F2-7 | 运行器：`go run` / `node` / `python3` / `bash` / `sh`；没有就显示"未配置运行器" | P1 |
| F2-8 | 与终端联动："在终端里运行"把命令 `term.Paste`/`Send` 给指定终端卡片 | P2 |

实现要点：
- 编辑器的 `Font.Size` 直接取 `12 * scale`，所以缩放后字仍清晰；改字号会让 `editor` 重新排版，需在缩放停下来后再改（见 R8）。
- 写盘与读取都在 goroutine，结果用 `win.Update` 回主线程（立即模式不许在 view 里做 I/O）。
- `editor.State` 是持久状态（选区、折叠、滚动），**放进节点的 runtime，不要放 view 局部变量**。

## 6. 需求 3：终端

### 6.1 接入

```go
rt.term, err = terminal.New(terminal.Options{
    Dir:     d.root,                                  // 工作区根
    OnTitle: func(t string) { w.Update(func(){ n.Title = t }) },  // 终端 goroutine → 主线程
    OnExit:  func(code int) { w.Update(func(){ /* 标记退出 */ }) },
    OpenLink: func(string) bool { /* ⌘+点击 终端里的路径 → 画布开卡片 */ },
})
// view:
terminal.View(c, rt.term).Grow(1).FillWidth()
```

| 编号 | 需求 | 优先级 | 说明 |
|---|---|---|---|
| T-1 | 每张卡片一个终端，独立 shell、独立 cwd、独立 scrollback | P0 | `terminal.New` 一次一个 |
| T-2 | 点击聚焦，输入法/组合键/`⌘C`（有选区复制，否则发给进程） | P0 | 插件内建 |
| T-3 | 退出显示 `[进程已退出 code]` + 重启按钮 | P0 | `OnExit` + `Done()` |
| T-4 | `⌘+点击` 终端里的 `path:line` / OSC 8 链接 → 打开文件卡片 | P1 | `OpenLink` |
| T-5 | 标题跟随 shell（OSC 0/2），cwd 跟随 OSC 7 | P1 | `Title()` / `Dir()` |
| T-6 | 缩放策略：终端字号**不跟随画布缩放**（避免反复 reflow 改变 PTY winsize），低于 LOD 阈值只留框 | P0 | 见 R2/R3 |
| T-7 | 主题：跟随窗口明暗（`terminal.DarkTheme()` / Ghostty 主题名） | P2 | |
| T-8 | 快捷键仲裁：画布 `⌘0/⌘+/-`、`⌘S` 优先；其余键进终端 | P0 | `c.Shortcut` 先于终端 |
| T-9 | 关闭窗口 → `Close()` 全部终端（SIGHUP），无残留进程 | P0 | ✅ 已接 `OnClosed` |
| T-10 | 平台：终端需要 macOS 13+（libghostty-vt），`mygo.json` 已设 `minimumSystemVersion` | P0 | ✅ |

## 7. 数据与持久化

```jsonc
{
  "version": 1,
  "root": "/Users/nonnika/Code/BoundlessDeck",
  "camX": 0, "camY": 0, "camS": 1,
  "blocks": [
    { "id": "n1", "kind": "note", "title": "BoundlessDeck", "x": 80, "y": 80, "w": 400, "h": 210, "text": "..." },
    { "id": "n2", "kind": "file", "title": "main.go", "x": 560, "y": 80, "w": 560, "h": 420, "path": "main.go" },
    { "id": "n3", "kind": "terminal", "title": "终端", "x": 120, "y": 520, "w": 560, "h": 340 }
  ]
}
```

- 存放在 `App.Path(PathUserData)/deck.json`；原子写；`version` 字段 + 迁移函数。
- 运行时状态（编辑器、位图、终端）**不进 JSON**，重启后重建；终端内容不做快照（v1）。
- 窗口位置尺寸由 `StateKey: "main"` 记住。

## 8. 非功能需求

| 编号 | 需求 |
|---|---|
| N-1 | 冷启动到可交互 ≤ 1s（纯 Go，无 webview 初始化） |
| N-2 | 崩溃隔离：I/O 全在 goroutine，`win.Update` 回主线程；单卡片失败只影响该卡片 |
| N-3 | 每帧预算：build+layout ≤ 8ms（1000 卡片 + 8 终端以外），用 `MYGO_FRAME_STATS=1` 复核 |
| N-4 | 可访问性：卡片 `.Access(...)` 描述；键盘 Tab 遍历 + 方向键微移 |
| N-5 | i18n：文案集中，中文/英文两套 |
| N-6 | 隐私：不上报路径与内容；预览默认不联网 |
| N-7 | 打包：`go tool mygo build` → `build/<os>-<arch>/`；macOS 13+ |

## 9. 里程碑

| 阶段 | 内容 | 出口 | 状态 |
|---|---|---|---|
| **M0** | 画布骨架：相机/缩放锚点/平移/框选/卡片拖动/LOD/持久化/测试 | 空跑不崩、缩放手感对、刷新不丢 | ✅ 本期完成 |
| M1 | 文件预览：markdown、失效态、watch 刷新、大文件、外部打开 | 10 类文件全绿 | 进行中 |
| M2 | 代码卡：双向写盘、冲突提示、运行器与输出 | 外部改动不丢 | 待做 |
| M3 | 终端：多卡片、链接互通、退出重启、快捷键仲裁、压测 | 8 终端 55fps、无残留进程 | 待做 |
| M4 | 打磨：撤销重做、resize、小地图、搜索、菜单、i18n、签名打包 | 端到端演示 | 待做 |

## 10. 风险与验证

| 编号 | 风险 | 验证 | 退路 |
|---|---|---|---|
| R1 | 立即模式重建开销随卡片数线性增长 | `MYGO_FRAME_STATS=1` + 1000 卡片压测 | LOD 阈值上调、`Cache()` 缓存卡片底、只重建脏卡片 |
| R2 | 画布缩放 vs 终端滚轮/字体缩放抢事件 | 在终端上捏合/滚轮实测 | 画布快捷键优先 + 终端内显示"按 ⌘ 缩放画布"提示 |
| R3 | 终端 `SetFont` 连带改变 PTY winsize，全屏程序重排抖动 | `vim`/`htop` 下持续缩放 | 字号只在缩放停顿 300ms 后按档位（1、1.25、1.5、2）变更 |
| R4 | `ui.WebView` 不可缩放且重 | PDF/HTML 预览实测内存 | 固定缩放面板 / 系统打开 |
| R5 | 视频无内置解码器 | `ui.Video` 试接 | v1 交系统播放器 |
| R6 | 对话框必须在主线程外调用 | 已在 `pickFile` 用 goroutine + `Update` | 失败则改菜单项触发 |
| R7 | `float32` 世界坐标在超大范围漂移 | 相机推到 ±1e5 观察 | 坐标限幅 / 相机原点定期归一化 |
| R8 | 改字号导致编辑器光标与滚动跳动 | 缩放连续操作 | 缩放期间冻结字号，停手后应用 |
| R9 | 每卡片一个终端的内存 | 8 终端实测 | 上限提示 + 闲置终端 `Close` 保留 scrollback 文本 |

## 11. v1 范围外

云同步与协作、插件市场、SSH/远程终端（可用 `terminal.Conn` 接后续版本）、Office 转换、Notebook 内核、连线语义、移动端。

## 12. 当前仓库状态（本期改动）

```
main.go       窗口 + Content: ui.View(d.view) + StateKey + 关窗保存
canvas.go     view / 网格 / 输入仲裁（平移、缩放、框选、卡片拖动、快捷键）/ 工具栏 / 状态栏
deck.go       节点模型、相机、路径归一化、原子持久化、种子卡片
blocks.go     卡片：标题栏、note、file（文本走 editor、图片走 bitmap）、terminal（懒启动）
canvas_test.go 8 个 Tester 单测：缩放锚点、缩放上下限、滚轮平移、⌘+滚轮缩放、标题拖动、正文点选、框选、LOD、持久化往返
snapshot_test.go 渲染回归快照 → docs/canvas-snapshot.png
mygo.json     纯原生项目的配置（无 frontend/bindings），macOS 13+
go.mod　      mygo v0.7.4 + `tool github.com/egoist/mygo/cmd/mygo`
```

已删除：`index.html`、`src/`（TS 前端与生成的绑定）、`vite.config.ts`、`tsconfig.json`、`package.json`、`mygo.config.ts`。

验证：`go build ./...`、`go test ./...`、`go test -race ./...`、`go tool mygo vet .` 全部通过；`/tmp/bd-native` 启动 6 秒无崩溃；`docs/canvas-snapshot.png` 为 1100×700 的无头渲染结果。