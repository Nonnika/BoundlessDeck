# Repository Guidelines

面向 AI 助手的操作手册。产品功能与 Go 版差异见 `README.md`；这里只讲代码结构、不变量与验证方法。

**术语**：画布上的每个元素都叫 **block**，代码、注释与 JSON 字段（`"blocks"`）都不出现 `node` / `card`。block 种类：`note`（QuickLook 便签）、`memo`（笔记，Markdown 文件）、`file`、`terminal`、`browser`、`chart`、`stack`（块堆）、`text`（文字）、`shape`（图形）。

## Project Overview

BoundlessDeck 的 Rust 重写：无限画布工作台，界面由 GPUI 原生绘制（应用 chrome 不用 WebView；`browser` block 内嵌 WKWebView）。一个 deck 就是一个文件夹，画布保存在 `<folder>/.boundlessdeck/deck.json`，格式与 Go 版兼容，两版可以互开。

同一个二进制有两种角色：
- **GUI**：默认启动，首个位置参数是要直接打开的文件夹。
- **MCP stdio 桥**：`boundlessdeck mcp-stdio [--workspace DIR]`。在 `main.rs` 里于 GPUI 启动前被拦截，把 stdio 转给正在运行的 deck 的 Unix socket。

## Architecture & Data Flow

### 分层

| 层 | 文件 | 规则 |
|---|---|---|
| 模型 | `src/model.rs` + `src/model/{block,camera,persist}.rs` | 纯状态：blocks、camera、selection、undo/redo、deck.json、持久化。不依赖 `Window`，可 headless 测试。 |
| 视图 | `src/deck.rs` + `src/deck/*.rs` | `DeckView` 持有 `Model` 与交互状态；输入、渲染、保存调度都在这里，按职责拆在 `deck/` 的子模块里（见 Key Directories）。 |
| 外壳 | `src/app.rs` + `src/app/` | `Root` 在主页（`app/homescene.rs`）与 deck 之间二选一；管理 recents、设置、打开/关闭文件夹；引导页 `app/welcome.rs`、主页与 deck 切换的模糊过渡 `app/stage.rs`、`app/backdrop.rs`。 |
| 窗口 | `src/main.rs`、`src/app/windows.rs`、`src/app/commands.rs` | 装配窗口与生命周期；设置窗口（⌘,）与快捷键窗口（⌘/）共用 `app/commands.rs` 的命令表。 |
| 界面部件 | `src/ui.rs` + `src/ui/` | `editor.rs`（文本编辑器）、`theme.rs`（颜色与调色板）、`icons.rs`、`font.rs`（字体索引）、`tape.rs`。 |
| Block 内容 | `src/content.rs` + `src/content/` | `terminal.rs`、`markdown.rs` + `mdlive.rs`、`chart.rs` + `chartview.rs`、`pdf.rs`、`word.rs`、`preview.rs`、`memo.rs`、`lifespan.rs`：每种内容的解析、渲染与 I/O。 |
| 平台 | `src/platform.rs` + `src/platform/` | macOS 原生集成：`nativepanel.rs`（系统颜色面板与取色器）、`pinch.rs`（触控板捏合）、`screenshot.rs`（窗口截图）、`layerzoom.rs`（在 GPU 上缩放窗口画面）、`webfocus.rs`（网页与 GPUI 的键盘仲裁）。 |
| MCP | `src/mcp/` | `mod.rs` 端点与注册表，`server.rs` socket 服务，`rpc.rs` JSON-RPC，`tools.rs` + `tools/` 字段表与工具，`bridge.rs` stdio 桥，`config.rs` 各 agent 的全局/项目配置。 |

### 帧模型（最容易出错的地方）

GPUI 每帧重建整棵元素树，所以：
- **元素（`Div`、`AnyElement`）只在本帧有效，不能存进结构体。** 跨帧状态放在 `DeckView.rts: HashMap<String, Runtime>`（`deck.rs:250`，每个 block 的缓存，永不落盘），或放进 `Entity`。
- 慢 I/O 不在 UI 线程做：`cx.background_spawn(...).detach()`，结果用 `this.update(cx, |d, cx| { …; cx.notify() })` 回到 UI 线程。
- 延迟与轮询用 `cx.background_executor().timer(dur)` 自调度，而不是在每次输入时重启计时器。
- LOD：`cam.s < LOD_SCALE` 时只画框和标题，不构建编辑器、终端、位图。

### 输入仲裁

- 按下事件由最内层元素处理，处理后调用 `cx.stop_propagation()`（例如 `on_card_down`，`deck/input.rs:187`）。需要挡住下层命中的元素用 `.occlude()`（GPUI 设为 `HitboxBehavior::BlockMouse`）。
- 画布的 move/up/wheel 监听挂在 window 级（`deck.rs` 的 `Render` 里，约 650–675 行），拖动可以越过其他 block。
- 滚轮/捏合在 block 内不被消费时冒泡到画布：⌘/⌃+滚轮缩放，否则平移。
- 焦点：block 获得或失去焦点只走 `DeckView::focus_block`（结束别处的编辑、键盘交给终端或画布）。失焦不隐藏、不重载、不重绘内容，各种 block 不自己处理失焦。键盘不能落空：`webfocus::guard` 每帧（及每次按键前）在窗口自己或看不见的网页拿着键盘时交还给 GPUI 视图；deck 把焦点交给自己的东西（`focus_block`、`start_editing`）时调用 `webfocus::release()`，让之前点进的网页交出键盘。原生网页只在手势真的改变画面时隐藏（`gesture_covers`：选框拉开并盖住它，或画布确实平移了），单击画布不隐藏。

### 缩放

- 不用 transform 缩放：所有坐标乘 `cam.s` 后重新布局，文字保持清晰。
- 缩放像放大照片（`deck/photozoom.rs` + `platform/layerzoom.rs`）：捏合或 ⌘+滚轮时不重新排版，而是由 Core Animation 在 GPU 上缩放、平移 GPUI 画好的那一帧（给窗口的 CAMetalLayer 设仿射变换，p' = k·p + t）；期间 GPUI 若要画帧，`render` 用画面拍下时的相机（`Photo::drawn`）画。工具栏、状态栏、面板与画布同在一个图层，缩放时隐藏，结束后淡入（`chrome_alpha`）。放大超过 1.55×、视野走出照片的范围、以及手势停下 150 ms 后，换成清晰重画的一帧（缩小时照片以鼠标为中心多画 1.5 倍的范围，`picture_for`，边缘不露空白，且不低于 `LOD_SCALE`）：`layerzoom::present` 在同一个 CATransaction 里让图层立即重绘（GPUI 的 `displayLayer:` 以 presentsWithTransaction 同步提交）并撤掉变换，所以不闪；它会跑一帧 GPUI，只能在 task 里、任何 `update` 之外调用。缩小时露出的窗口边缘是窗口背景色，`attach` 把它设成画布背景。镜头滑动（`set_view`、焦点放映）仍逐帧排版。其余内容一律按 `content_scale()`（即 `cam.s`）排版；PDF 位图按 `DeckView.lay` 生成，缩放停下后才重做。`BOUNDLESSDECK_FRAME_STATS=1` 可在每次手势结束后打印帧耗时。
- 布局几何（位置、大小）仍用原始 `cam.s`。
- 清晰度过渡（`src/deck/sharpen.rs`）：内容换成更清晰的版本时统一淡入（180 ms，ease-out；系统开启“减弱动态效果”时直接切换）。矢量内容只在从 LOD 回到完整内容时过渡：LOD 下 body 区是与 block 背景同色的遮罩，回来时内容立即完整画出、遮罩淡出（450 ms，ease-in-out，`REVEAL_TIME`；`Runtime::appear`，`block_view` 统一处理）；位图（PDF）新页在拉伸的旧页上淡入（`Runtime::sharpen` + `crossfade`）；原生视图（WKWebView）露出时快照留在下面，视图 alpha 从 0 到 1。新增 block 种类时归入这三类之一，不要自己写时长与曲线。任何过渡进行中要设 `DeckView.animating`，由 `Render` 统一请求下一帧。
- 出现动画（`src/deck/popin.rs`）：只有 `on_create` 的 block（撤销恢复、打开 deck 不算）在第一次被画出时展开：亮线从中间横向拉开、上下打开、扫描线自上而下印出内容，约 560 ms；期间原生网页先不显示。系统开启“减弱动态效果”时直接出现。
- 相机滑动：`set_view` 先把目标相机写盘，再由 `DeckView.glide` 在 `Render` 开头逐帧推进（520 ms，远距离时中途拉远）；人的平移、滚轮、缩放会打断它。

### 撤销

- `Change`（`model.rs:69`）：`Add`、`Remove`、`Moves`、`Stack`、`Group`。一次用户操作 = 一条 `Change`（复合操作用 `Group`）。
- 进入画布只走 `Model::add_block`，离开只走 `Model::remove_block`（或经由 `detach`）。移动与排版用 `record_moves` / `apply_placements`，不要直接改 `b.x` / `b.y`。
- 块堆只改 `Block.items`，被收进的 block 仍在 `model.blocks` 中，由 `visible()` 与 `select_in` 跳过。

### 生命周期

`src/deck/lifecycle.rs`。四对事件，内层只在外层之内，离开外层前先离开内层：

| 层 | 事件 | 何时 |
|---|---|---|
| 数据 | `on_create` / `on_delete` | 第一次被做出来（只有 `Model::add_block`，撤销恢复不算）/ 离开画布（可撤销，只放运行时，不碰 memo 文件等外部数据） |
| 画布 | `on_open` / `on_close` | deck 打开、新建或撤销恢复后进入 / deck 关闭（保存之前） |
| 显示 | `on_resume` / `on_pause(Pause)` | 完整显示 / 屏幕外、收进块堆、LOD、窗口非活动；shell 暂停时照常运行 |
| 焦点 | `on_focus` / `on_blur(Blur)` | 成为正在操作的 block / 不再是；`focus_target`（编辑中的或 `active`）记着焦点该在谁，暂停失焦后恢复时自动回来，`Blur::Paused` 保留编辑器 |

- 派发只在 lifecycle.rs：`life_tick` 每帧比对状态（在 `Render` 里画 block 之前），`life_delete` 消费 `Model::hung_up`（离开的 `Block` 快照），`life_close` 由 `DeckView::close` 调用。状态存在 `Runtime.life`。
- 每种 kind 实现 `BlockHooks`（全部默认空）：终端在首次 `on_resume` 起 shell、`on_focus` 拿键盘、`on_blur` 清残留组字、`on_close`/`on_delete` 挂断；memo 在 `on_pause`/`on_close`/`on_delete` 写回文件。新增 kind 时在 `hooks()` 里登记，不要在渲染里懒启动运行时。
- 网页的显示/隐藏仍在 `browser_body` 每帧按遮挡判断，不是生命周期事件。

### 保存

- `Model::save` → `write_file_atomic`（`model/persist.rs:168`）：临时文件、`sync_all`、rename。
- `DeckView::request_save`（`deck.rs:398`）：在 UI 线程生成快照，后台线程写盘；`save_seq: Arc<Mutex<u64>>`（`deck.rs:277`）保证只有最新快照落地。

### MCP 数据流

```
外部 agent ──stdio──▶ boundlessdeck mcp-stdio ──Unix socket + token──▶ server.rs
                                                                         │ JSON-RPC
                                                    rpc.rs / tools.rs ◀──┘
                                                          │ 任务投递
                                       DeckView::start_mcp（deck/endpoint.rs:13）→ run_job（:85）
                                          在 UI 线程执行，15 s 超时后返回错误
```

- socket 路径：`<root>/.boundlessdeck/mcp.sock`；路径太长时回退到 `$TMPDIR/boundlessdeck-mcp-<hash>.sock`（`mcp/mod.rs`）。
- 全局配置（设置窗口里一键安装，`mcp/config.rs` 的 `agents_in`）：Claude Code、Claude 桌面版、Cursor、VS Code（JSON），Codex（`~/.codex/config.toml` 或 `$CODEX_HOME`，用 `toml_edit` 保留注释与排版，`env_vars` 放行终端变量），OpenCode v2（`~/.config/opencode/opencode.json(c)` 或 `$XDG_CONFIG_HOME`，`mcp.servers`、`type: local`、`environment` 用 `{env:NAME}`；含注释的 JSONC 只报告不改），oh-my-pi（`~/.omp/agent/mcp.json` 或 `$PI_CODING_AGENT_DIR`，`env` 按变量名传递）。桥会忽略未设置时留下的占位值（变量名本身、`{env:…}`）。
- 项目配置（`.boundlessdeck/mcp.json`、`.mcp.json`、`.cursor/mcp.json`、`.vscode/mcp.json`）以合并方式写入，保留用户已有的 server；坏 JSON 备份为 `.bak`。
- 写入类工具先干跑校验整批，再统一落笔；所有写操作都经过撤销栈与保存。
- **kind 名在 MCP 与 deck.json 中不同**：MCP 里笔记叫 `note`、QuickLook 叫 `quicklook`（`BlockKind::mcp_name` / `parse_mcp`，旧名 `memo` 仍当作笔记）；deck.json 仍存 `memo` / `note`（`as_str`），以便与 Go 版互开。
- **字段唯一来源**：`src/mcp/tools/schema.rs:31` 的 `pub const FIELDS: &[Field]`。`block_schema`、校验（`tools/patch.rs`）、工具描述都从这里生成。新增 `Block` 字段必须登记到这张表。
- 工具（`tools.rs` 的 `Ctx::call` 分派，约 490–550 行；排版工具在 `tools/arrange.rs`）共 24 个：`deck_info`、`block_schema`、`list_blocks`、`get_block`、`read_selection`、`current_terminal`、`screenshot`（截取窗口里的画布，PNG 以 MCP 图像内容返回）、`create_block`、`update_block`、`update_blocks`、`delete_blocks`、`move_blocks`、`align_blocks`、`distribute_blocks`、`arrange_blocks`、`select_blocks`、`set_view`（移动视野，窗口平滑滑过去）、`process`（agent 报告任务进度：准备中/进行中/已完成，窗口底部中间显示胶囊，完成后消失；见 `deck/progress.rs`，服务器 instructions 要求 agent 必须用它）、`focus_list`、`focus_set`、`focus_play`（焦点与焦点线的读写与放映，见 `tools/focus.rs`；放映状态在窗口里，工具经 `Ctx.playing` 读、`Ctx.play` 下指令）、`undo`、`redo`、`save_deck`。
- 调用方的终端：终端 shell 的环境里有 `BOUNDLESSDECK_TERMINAL`（block ID）与 `BOUNDLESSDECK_DECK`（deck 文件夹），见 `mcp::TERMINAL_ENV`。从终端里启动的 agent 再启动 `mcp-stdio` 时继承它们，桥用 `X-BoundlessDeck-Terminal` / `X-BoundlessDeck-Deck` 请求头带给 `server.rs`，经 `DeckRunner` 的 `Caller` 进 `tools::Here`。`current_terminal` 返回 `self`（调用方所在终端）与 `focused`（人正在操作的终端，`focus_target`），`deck_info` 带 `callerTerminal`。
- 资源：`boundlessdeck://deck`、`//blocks`、`//schema`，以及模板 `//blocks/{id}`（`rpc.rs:38–72`，读取见 `tools.rs:477`）。

### 运行时状态位置

| 状态 | 位置 |
|---|---|
| deck 数据 | `<folder>/.boundlessdeck/deck.json` |
| MCP 配置 / schema 描述 | `<folder>/.boundlessdeck/{mcp.json,block-schema.json}` |
| 最近列表、设置 | `BOUNDLESSDECK_DATA_DIR` 或平台数据目录（`model/persist.rs:160`） |
| 活动 deck 注册表 | `BOUNDLESSDECK_ACTIVE_FILE` 或 `dirs::config_dir()/BoundlessDeck/active-deck.json`（`mcp/mod.rs:103`） |
| MCP 文件权限 | 写入后 `chmod 0600`（`mcp/mod.rs` 的 `write_private`） |

## Key Directories

| 路径 | 内容 |
|---|---|
| `src/` | 唯一的 crate（`boundlessdeck`，single binary，无 `[lib]`）。顶层只有 `main.rs` 与七个模块：`app`（外壳）、`ui`（界面部件）、`content`（block 内容）、`platform`（macOS 集成）、`deck`、`model`、`mcp`。分组模块是同名的 `x.rs` 加 `x/` 目录，子模块用 `pub(crate) mod` 声明，引用写全路径（`crate::ui::theme::Theme`）。 |
| `src/deck/` | `DeckView` 的子模块：`input.rs`（按下、拖动、滚轮与焦点）、`card.rs`（block 卡片、标题栏、缩放手柄、出现动画）、`body.rs`（卡片内容与 Markdown/图表缓存）、`chrome.rs`（工具栏、主页按钮、弹层、状态栏）、`camera.rs`（缩放与相机滑动）、`editing.rs`（原地编辑）、`memofile.rs`（笔记文件读写）、`load.rs`（文件、文档、PDF、终端的加载）、`endpoint.rs`（MCP 端点与任务执行）、`stack.rs`（块堆与弹簧动画）、`browser.rs`（WKWebView 嵌入与快照）、`zoomlayer.rs`（缩放状态机）、`sharpen.rs`（清晰度过渡）、`lifecycle.rs`、`popin.rs`、`progress.rs`（agent 任务进度胶囊）、`photozoom.rs`（照片式缩放）、`focusview.rs`（焦点视图、焦点面板与放映）、`styled.rs`、`inspector.rs` |
| `src/model/` | `block.rs`（`BlockKind`、`Block`、`Style`、默认尺寸）、`camera.rs`（相机与缩放档位）、`focus.rs`（焦点与焦点线）、`persist.rs`（路径与 deck.json 读写）；都由 `model.rs` 重新导出，外部仍写 `crate::model::X` |
| `src/mcp/` | MCP 端点，见上文；`tools/` 下是 `schema.rs`（字段表与工具列表）、`patch.rs`（字段校验与写入）、`arrange.rs` + `layout.rs`（排版工具与算法）、`style.rs`（样式补丁） |
| `target/` | 构建产物，已 gitignore |

## Development Commands

```sh
cargo run                                   # 主页
cargo run -- /path/to/folder                # 直接打开该文件夹的 deck
cargo run -- mcp-stdio --workspace DIR      # MCP stdio 桥（无 deck 时也能握手）
cargo test                                  # 全部单元测试（75 个）
cargo test model::tests                     # 按模块路径过滤
cargo test -- --test-threads=1              # socket / 计时相关测试排查时串行运行
cargo build --release
cargo bundle --release                      # 打包 target/release/bundle/osx/BoundlessDeck.app（需 cargo install cargo-bundle --locked；图标 resources/icon.icns）
cargo clippy                                # 有现存警告（见下），不要新增
```

试运行时隔离真实数据：

```sh
export BOUNDLESSDECK_DATA_DIR="$(mktemp -d)"
export BOUNDLESSDECK_ACTIVE_FILE="$(mktemp -d)/active-deck.json"
cargo run -- /tmp/test-deck
```

环境变量（已在代码中核对）：

| 变量 | 作用 | 位置 |
|---|---|---|
| `BOUNDLESSDECK_DATA_DIR` | 覆盖数据目录（recents、settings） | `model/persist.rs:160` |
| `BOUNDLESSDECK_ACTIVE_FILE` | 覆盖活动 deck 注册表路径 | `mcp/mod.rs:103` |
| `BOUNDLESSDECK_WORKSPACE` | `mcp-stdio` 的默认工作区 | `mcp/bridge.rs:98` |
| `BOUNDLESSDECK_TERMINAL` / `BOUNDLESSDECK_DECK` | 终端 shell 中由应用设置；`mcp-stdio` 读取后告诉 deck 调用方在哪个终端里 | `mcp/mod.rs`、`content/terminal.rs` 的 `spawn` |

### 工具链现状

- **格式**：仓库没有 `rustfmt.toml`，`cargo fmt --check` 报告约 583 处差异（约 9900 行输出）。不要整文件格式化，只让改动部分符合周围风格。
- **Lint**：没有 `clippy.toml` 或 `[lints]`。当前 clippy 警告包括 `collapsible_if` 与 `redundant_locals`（`main.rs:88`）。新代码不要新增警告。
- **工具链**：无 `rust-toolchain` 文件，也没有 `rust-version` 字段。edition 2024 需要 Rust 1.85+（外部事实，仓库未声明）。本机观察到 rustc 1.99。
- **`gpui` 特性**：`runtime_shaders` 已开启，因为本机没有 Xcode 的 `metal` 编译器。不要移除。

## Code Conventions & Common Patterns

### 命名与语言

- Block ID：`b` + 6 位十六进制，由 `Model::new_id` 生成，不复用。
- 模块文件名为单数（`model.rs`、`ui/editor.rs`），子模块放进同名目录（`deck/`），不用 `mod.rs`。新文件放进对应分组（`app`、`ui`、`content`、`platform`），不要放回 `src/` 顶层。
- 模块级文档注释（`//!`）解释设计理由，写中文或英文都可，但保持该文件已有的语言。用户可见文案（错误、按钮、状态）用简体中文。
- 提交信息：Conventional Commits，scope 与主题用中文。仓库里的例子：`feat(canvas): 支持触控板捏合缩放`、`feat(browser): 网页隐藏时以 WebKit 快照代替`。

### 错误处理

- 模型与 MCP 边界统一 `Result<T, String>`，错误文案中文，直接返回给 agent 或显示在 block 里。
- `anyhow` 仅在 `Terminal::spawn` 使用。
- 运行时错误存为 `Option<String>` 或 `Result` 字段（如 `term_err`、`web_err`），渲染在 block 内，不弹窗。
- 不在 UI 线程 `unwrap` 用户数据；内部不变量（刚插入的 map 项、锁）可以 `unwrap`。

### 异步

```rust
cx.spawn(async move |this, cx| {
    // await 通道或定时器
    let _ = this.update(cx, |d, cx| { /* 修改状态 */ cx.notify(); });
})
.detach();
```

- 阻塞 I/O：`cx.background_spawn(async move { … }).detach()`。
- 原生回调（WKWebView、AppKit 事件）：推进 `smol::channel`，由一个 `cx.spawn` 循环在 UI 线程消费（见 `browser.rs` 的 `make_webview`，`deck.rs` 的 web 事件泵）。
- 所有 spawn 结果都要 `.detach()`，并允许 `this.update` 失败（实体已释放）时静默返回。

### 状态与依赖注入

- 视图是实体：`cx.new(|cx| DeckView::new(…))`，父级用 `cx.subscribe` 监听子级发出的 `DeckEvent`（`Home` / `Open` / `Pick`）。
- 应用级句柄用 GPUI global（如 `windows::RootHandle`）。
- 没有 DI 容器：依赖通过构造参数传入。纯逻辑（camera、selection、布局、解析）写成自由函数或 `Model` 方法，便于测试。
- 模块可见性：默认私有，子模块需要时用 `pub(super)`（如 `zoomlayer` 的 `zooming`）。

### 渲染与缓存

- 布局用 `div()` 与 flex；大量文字用 `gpui::list` / `uniform_list`，并把列表高度限制在窗口能看到的部分（`DeckView::body_room`）：GPUI 的列表按自身高度而不是窗口决定排版到哪，放大到长文档上时否则会把窗口外的段落全排一遍；画在别处的正文（块堆预览）关掉这个裁剪（`clip_bodies`）；点阵、图表、胶带、主页画布用 `canvas(…)` 中的 `paint_quad` / `paint_path`。
- 派生结果按输入缓存（Markdown 解析、图表数据、PDF 页位图）；缓存键不要包含连续的缩放值，PDF 位图按 `lay.s` 生成。
- 颜色来自 `Theme`，不要硬编码。

### 平台代码

- macOS 专属代码用 `#[cfg(target_os = "macos")]` 包裹：`content/pdf.rs`（CoreGraphics 的 `#[link]` 与 extern 块）、`platform/` 下的各文件（AppKit、ScreenCaptureKit）、`deck/browser.rs`（WebKit 快照，objc2）。
- 非 macOS 的降级路径应返回用户可读的错误字符串，不 panic。

## Important Files

| 文件 | 为什么重要 |
|---|---|
| `src/main.rs` | 入口：MCP 桥分派、窗口选项、关闭与退出钩子。 |
| `src/model.rs` | Change、撤销、块堆、选择等模型操作；block、camera、持久化在 `model/` 子模块。行为修改从这里开始。 |
| `src/deck.rs` | 画布视图：`DeckView` 与 `Runtime` 的定义、保存调度、增删 block、`Render`；输入、卡片、内容、MCP 任务执行在 `deck/` 子模块。 |
| `src/deck/zoomlayer.rs` | 缩放进行中的判断与 PDF 光栅比例（`lay`），帧耗时统计。 |
| `src/deck/browser.rs` | WKWebView 的生命周期、遮挡与快照回退。 |
| `src/mcp/tools.rs` | `Ctx` 与 24 个工具的分派；字段表、校验、排版在 `tools/` 子模块。 |
| `src/mcp/mod.rs` | 端点注册、socket 路径、活动 deck 注册表。 |
| `src/app/commands.rs` | 命令表：菜单、快捷键与快捷键窗口的单一来源。 |
| `Cargo.toml` | 依赖；`gpui` 0.2.2 与 `runtime_shaders`；macOS 专属 objc2 系列。 |
| `README.md` | 功能清单与 Go 版差异。 |

### 常见改动路径

- **浏览器标签页**：`Block.tabs`（`WebTab`，`key` 只在运行时区分标签、不落盘）与 `Block.tab`；`url` 始终等于打开的标签的地址。只写 `url` 的地方（Go 版、MCP、地址栏）之后调用 `Block::ensure_tabs` 让打开的标签跟上。每个标签的 WKWebView 存在 `Runtime.webs`（按 key），`WebEvent` 都带 block id 与标签 key。
- **新增 block 字段**：在 `Block` 上加 serde 字段（`skip_serializing_if` 保持与 Go 版一致）→ 登记到 `mcp::tools::FIELDS`（`tools/schema.rs`）→ 在 `tools/patch.rs` 中实现读写校验。
- **新增 block 种类**：`BlockKind`（含 `ALL`、`as_str`、`parse`、`label`）、`new_block_of` 与 `kind_defaults` 的默认尺寸、`deck/body.rs` 中的 body 渲染分支、`FIELDS` 中的 `kinds`。旧版本读到不认识的种类时把它原样留在 `Model.foreign` 并写回，所以新种类不会让旧版本打不开 deck；deck.json 整体读不了时 `Model::load` 返回错误，不会新建一张覆盖它。
- **文字与图形**（`BlockKind::bare()`）：没有卡片，由 `deck/styled.rs` 画（`look` 把 `Block.style` 解析成颜色、字体、对齐，缺省按种类）；选中时标题栏浮在上方，单击拖动、双击编辑文字。样式在右侧面板（`deck/inspector.rs`）里改：面板从右侧滑入滑出（拖影模拟动态模糊，`DeckView.insp` 记着进度）；字体在面板旁自己的列表里选，索引由 `ui/font.rs` 在启动时于后台用 Core Text 建好（含本地化名）；字号与圆角可加减也可直接输入，圆角可四角分别设置（`Style.corners`）；自定义颜色走系统面板（`src/platform/nativepanel.rs`，选择经 `DeckView.panel_tx` 回到 UI 线程）。
- **焦点**：`Model.focus`（列表顺序即默认放映顺序）与 `Model.focus_lines`（`stops` 是焦点 ID，可重复），存在 deck.json 顶层的 `focus` / `focusLines`；焦点存的是视野中心的世界坐标与缩放，`Focus::camera(view)` 换成相机。deck.json 顶层不认识的字段放在 `Model.extra` 原样写回。焦点的增删改不进撤销栈。焦点视图与放映的交互都在 `deck/focusview.rs`（`DeckView.fv`），放映复用 `glide_to`。
- **新增命令**：`app/commands.rs` 的命令表，并在对应的 key context 上挂 `.on_action` 处理。
- **新增交互**：在最内层元素上 `.occlude()` 或 `stop_propagation()`；拖动状态放在 `Drag` 中，结束时用 `record_moves` 与 `after_edit`。

## Testing & QA

- **框架**：标准 `#[test]`，75 个，分布在 24 个带 `#[cfg(test)]` 的源文件中。无 `tests/` 目录，无 GPUI 测试框架（`TestAppContext`）。
- **覆盖**：`model.rs`（相机、撤销、块堆、到期、持久化）、`mcp/`（工具、JSON-RPC、socket、配置合并、桥）、解析器（图表、Markdown、docx、PDF、终端按键）。
- **未覆盖**：GPUI 渲染与命中测试、真实 PTY、WKWebView 生命周期、`app.rs` 与 `ui/editor.rs` 的交互。
- **模式**：
  - 文件系统测试用 `tempfile`，不碰真实数据目录或 MCP 配置。
  - 断言可观察状态（`m.blocks`、`m.cam`、`m.sel`、撤销栈长度、文件内容），不要只断言"没 panic"。
  - 依赖系统能力的测试在能力缺失时提前 `return`，不用 `#[ignore]`。
- **必做的冒烟验证**（UI 与 MCP 改动）：
  1. `cargo test` 与 `cargo clippy` 无新增失败或警告。
  2. `cargo run -- /tmp/test-deck`，操作改动的路径，检查 `<folder>/.boundlessdeck/deck.json` 的内容。
  3. 改动 MCP 时，用 `cargo run -- mcp-stdio --workspace /tmp/test-deck` 通过 stdin 发送 JSON-RPC（`initialize`、`tools/list`、`tools/call`）并检查响应。
