# 插件（WebAssembly）

> 状态：方案，未实现。记录于 2026-11-08（在 2026-11-07 的插件 block 想法上扩展了状态监控与宿主 API）。

插件用 Rust（或任何能编到 wasm 组件的语言）编写，编译为 `wasm32-wasip2`，由 wasmtime 加载。插件能做两件事：

1. **注册 block 种类**：画布上出现新的 block，界面用声明式视图树描述，由宿主用 GPUI 原生绘制。
2. **监控 block 状态**：订阅画布上 block 的增删、移动、字段变化、选择与焦点，并可经宿主 API 读写画布。

一个插件可以只做其中一件，也可以都做。

## 结论

| 问题 | 选择 |
|---|---|
| 插件形态 | wasm 组件，接口用 WIT 定义（wasmtime Component Model）。Zed 的扩展同样做法，Zed 也是 GPUI 应用。 |
| 界面 | **声明式视图树**：插件返回可序列化的节点树，宿主翻译为 `div()` 等 GPUI 元素。不用 WebView。 |
| 画布读写 | 复用 MCP 工具层（`mcp/tools.rs`），插件与 agent 走同一条校验、撤销、保存路径。 |
| 状态监控 | 宿主对 block 快照做差分，产出事件；不在各个修改点埋点。 |
| 运行 | 每个插件一个实例，跑在后台线程，epoch 中断防死循环；与 UI 线程用 `smol::channel` 通信。 |

### 为什么不用另外几种

| 方案 | 问题 |
|---|---|
| Rust 动态库（dylib + `libloading`） | Rust 没有稳定 ABI：插件必须与应用同一编译器、同一版 GPUI，任一方升级即崩溃。GPUI 的元素与 `Context` 也不能跨库边界。 |
| 编进应用（插件 crate 实现 trait，开 Cargo feature） | 每加一个插件都要重新编译应用。适合内置 block，不算插件系统。 |
| 插件画 HTML，放进 WKWebView | 重；与“应用界面原生绘制”的原则冲突；还要进 `webfocus` 的键盘与滚轮仲裁，缩放时只能靠快照，文字不清晰。 |
| 插件直接输出像素位图 | 缩放时要么模糊要么每档重画，失去真实缩放；主题与 LOD 都得插件自己处理。 |

## 一、声明式视图树

### 节点

第一版的词汇（WIT 里是一个 `variant`，树用扁平数组 + 子节点下标表示，WIT 不支持递归类型）：

| 节点 | 属性 | 映射 |
|---|---|---|
| `col` / `row` | `gap`、`pad`、`align`、`justify`、`grow`、`scroll` | `div().flex()…`；`scroll` 用带 id 的 `overflow_y_scroll` |
| `text` | 文本、`size`、`weight`、`italic`、`color`、`mono`、`wrap` | `div().child(text)`；`mono` 用 `font::mono()` |
| `markdown` | 源文本 | 复用 `markdown::parse_doc` + `render_parsed`，缓存同 memo |
| `image` | deck 内相对路径或插件提供的字节 | `gpui::img` |
| `icon` | 内置图标名 | `svg().path("icons/<name>.svg")` |
| `button` | 文字或图标、`on`（事件 id） | 带 hover 的 `div`，`on_mouse_down` + `stop_propagation` |
| `input` | 当前值、占位、`multiline`、`on-change` / `on-submit` | 宿主的 `Editor` 实体（IME、光标、选择由宿主负责） |
| `checkbox` / `toggle` | `checked`、`on` | 同 Markdown 勾选框的画法 |
| `canvas` | 绘图指令：`rect`、`line`、`path`、`text`、`circle` | `canvas(…)` 中 `paint_quad` / `paint_path`，同 `chartview.rs` |
| `spacer` / `divider` | — | `flex_1()` / 1px 分隔线 |

颜色只能是**主题色名**（`text`、`muted`、`accent`、`danger`、`surface`、`border`…）或显式 RGBA。用主题色名时深浅色切换由宿主处理，插件不用感知。

不提供的：任意 CSS、绝对定位（`canvas` 内部除外）、动画曲线。过渡统一由宿主按 `sharpen.rs` 的三类处理（插件 block 属于矢量内容）。

### 与帧模型的关系

GPUI 每帧重建元素树，但**插件不按帧渲染**：

- 插件只在 `state`、`data`、尺寸（取 `content_scale` 档位，不是连续值）或主题变化时被调用 `render`。
- 返回的树存在 `Runtime.plugin_view`（带版本号），每帧由宿主从这棵树重建 GPUI 元素。这一步只是遍历，不调 wasm。
- 插件用逻辑单位（与 block 的 `w`/`h` 同一坐标系）。宿主乘 `content_scale()` 排版，布局几何用 `cam.s`，与其他 block 一致。
- LOD（`cam.s < LOD_SCALE`）时不翻译视图树，只画框和标题。
- `render` 在后台线程执行，期间继续显示上一棵树；第一次还没返回时显示“加载中…”。

### 事件

宿主把节点上的交互转成事件发给插件：

```
event = click(node-id) | change(node-id, string) | submit(node-id, string)
      | key(key, modifiers)   // 仅在 block 获得焦点且插件声明需要键盘时
      | scroll(dx, dy)        // 仅在插件声明 consume-scroll 时；否则冒泡到画布平移
      | timer(id)
```

插件 `on-event` 返回新的 `state`（以及可选的“需要重绘”）。按下事件在最内层节点上 `stop_propagation()`，规则同 AGENTS.md 的输入仲裁。

## 二、注册 block

### deck.json

```json
{
  "id": "b1a2b3c", "kind": "plugin", "title": "番茄钟",
  "x": 0, "y": 0, "w": 320, "h": 200,
  "plugin": "com.example.pomodoro/timer",
  "data": { "minutes": 25, "running": false }
}
```

- `kind` 统一为 `plugin`；`plugin` 是 `<插件 id>/<block 类型>`；`data` 是插件自己的 JSON 状态。
- 插件的运行时状态（计时中的剩余秒数等）放在实例里，不落盘；需要持久的放进 `data`。

### 前置改动：先能保留不认识的 block

**现状**：`BlockKind` 的反序列化遇到未知 `kind` 直接报错（`model.rs` 的 `impl Deserialize for BlockKind`），整个 deck 打不开。所以插件之前要先做：

- 新增 `BlockKind::Unknown`，原样保留整条 JSON（`#[serde(flatten)]` 的额外字段），保存时写回，不丢数据。画成一个只读占位框：“需要插件 X”。
- 先发一个带此改动的版本。之后的插件 block 被旧版本打开时仍能原样保留。
- Go 版的行为另行确认。至少要保证它不会删掉或改写这些 block。

### manifest（`plugin.toml`）

```toml
id = "com.example.pomodoro"
name = "番茄钟"
version = "0.1.0"
api = "0.1"                       # WIT 接口版本

[[block]]
type = "timer"
label = "番茄钟"
icon = "timer"
size = [320, 200]
schema = "timer.schema.json"      # data 的 JSON Schema，并入 MCP 的 block_schema

[permissions]
watch = ["terminal", "note"]      # 可订阅哪些 kind 的事件；"*" 为全部
read = ["text", "title"]          # 可读哪些字段；memo 正文、终端输出另行声明
write = true                      # 可否经宿主 API 改画布
fs = "deck-read"                  # none | deck-read | deck-write
net = ["api.example.com"]
```

插件位于 `~/Library/Application Support/BoundlessDeck/plugins/<id>/`（`plugin.wasm` + `plugin.toml`），工具栏与菜单列出它注册的 block 类型。每个 deck 可单独启用或停用。

### 生命周期

现有 `BlockHooks`（`deck/lifecycle.rs`）的四对事件原样导出：

| WIT 导出 | 对应 | 用途举例 |
|---|---|---|
| `on-create` / `on-delete` | 数据层 | 初始化 `data` / 清理外部资源 |
| `on-open` / `on-close` | 画布层 | 恢复计时 / 收尾 |
| `on-resume` / `on-pause` | 显示层 | 屏幕外停掉定时器 |
| `on-focus` / `on-blur` | 焦点层 | 申请键盘 |

宿主为 `plugin` kind 实现一个 `BlockHooks`，把调用转发到对应实例。派发仍只在 `lifecycle.rs`。

## 三、监控 block 状态

### 怎么产出事件

block 被修改的路径很多：`add_block` / `remove_block` / `record_moves` 是规范路径，但也有直接写字段的（编辑器的 `Changed` 改 `b.text`、勾选框切换、MCP 的 `update_block`）。memo 正文甚至不在 `Model` 里，在 `rt.memo.text`。逐个埋点必然漏。

做法：**差分快照**。

- `DeckView` 保留上一份 block 摘要：`HashMap<id, (kind, x, y, w, h, 各字段的 hash)>`。
- 每次 `request_save`（所有改动最终都会走到这里）以及每帧末尾的选择与焦点比对时，与当前状态差分，得出事件。
- memo 正文在 `save_memo` 成功后发 `content-changed`；终端输出单独限流（见下）。
- 同一 block 的事件在约 100 ms 内合并后投递，拖动过程中只发一次 `moved`（拖完再发）。

### 事件

```
block-event =
    added(block) | removed(id) | moved(list<(id, rect)>)
  | changed(id, list<field-name>)        // 只给字段名，内容用 get-block 取，按 read 权限过滤
  | content-changed(id)                   // memo 文件、终端输出
  | selection(list<id>) | focus(option<id>)
  | view(camera)                          // 可选订阅，节流到 4 Hz
```

插件导出 `on-block-event(list<block-event>)`，订阅条件用 `subscribe(filter)` 声明：按 kind、id 或字段过滤，超出 manifest 权限的部分忽略。

终端输出量大，不逐字节推送：`content-changed` 至多每秒一次，插件自己用 `read-terminal(id, last-n-lines)` 拉取。

## 四、宿主 API（插件可调用）

画布读写**直接复用 MCP 工具层**：`tools.rs` 的 `list_blocks`、`get_block`、`create_block`、`update_blocks`、`delete_blocks`、`move_blocks`、`arrange_blocks`、`select_blocks`、`set_view` 等以函数形式暴露给插件。这样可以白拿：

- 先干跑校验整批再落笔；
- `FIELDS` 仍是字段唯一来源；
- 撤销栈与保存。

其余能力：`data` 读写（仅本插件的 block）、`timer(ms)`、`log`、`http-fetch`（按 `net` 放行）、deck 内文件读写（按 `fs`）、`read-terminal`。

### 撤销

- 用户在插件 block 里点按钮改了 `data`：一次 `on-event` 的结果 = 一条 `Change`。需要新增 `Change::Fields { id, before, after }`。现状是 `Change` 只有 `Add`/`Remove`/`Moves`/`Stack`/`Group`，正文编辑也不进撤销栈，这里要一起想清楚。
- 插件在 `on-block-event` 里对画布的写入：并入触发它的那次用户操作（同一个 `Group`），⌘Z 一次撤回两者。
- 防循环：插件自己写入引起的事件不再回发给它自己；一次用户操作引起的插件写入链最多 3 层，超出丢弃并记日志。

## 五、运行与隔离

- 每个插件一个 wasmtime `Store`，跑在专用后台线程；UI 线程与其之间用 `smol::channel`，结果用 `this.update(cx, …)` 回到 UI 线程，同 `browser.rs` 的事件泵。
- epoch 中断：单次调用超过 50 ms 打断，记为超时；连续超时的插件停用，block 显示错误（存为 `Option<String>`，渲染在 block 内，不弹窗）。
- 内存上限（`StoreLimits`），默认 64 MB。
- WASI p2，默认无文件系统、无网络，按 manifest 授权；首次启用时展示权限。
- 编译结果缓存（`Module::serialize` 到数据目录），启动不重复编译。
- wasmtime 让二进制增大十几 MB、编译时间变长：放在 `plugins` Cargo feature 后面，默认开启。

## 六、插件作者的 SDK

crate 暂名 `boundlessdeck-plugin`，封装 `wit-bindgen` 生成的绑定与视图树构建函数：

```rust
use boundlessdeck_plugin::*;

#[derive(Serialize, Deserialize, Default)]
struct Counter { n: u32 }

impl Block for Counter {
    fn render(&self, _: Size) -> View {
        col([
            text(format!("{}", self.n)).size(32.).weight(Bold),
            row([button("减一").on("dec"), button("加一").on("inc")]).gap(8.),
        ])
        .gap(12.)
        .align(Center)
    }

    fn on_event(&mut self, ev: Event) {
        match ev.id() {
            "inc" => self.n += 1,
            "dec" => self.n = self.n.saturating_sub(1),
            _ => {}
        }
    }
}

struct Watcher;

impl Plugin for Watcher {
    fn subscribe() -> Filter { Filter::kind("terminal").events([Added, Removed]) }

    fn on_block_events(events: Vec<BlockEvent>) {
        for e in events {
            if let BlockEvent::Added(b) = e {
                host::log(&format!("新终端 {}", b.id));
            }
        }
    }
}

export_block!("counter", Counter);
export_plugin!(Watcher);
```

`Counter` 的 `self` 序列化后就是 deck.json 里的 `data`。

## 待决问题

- **WIT 接口的版本策略**：manifest 写 `api` 版本，宿主同时支持当前与上一个小版本？第一版只给自己用，可以先不承诺兼容。
- **视图树节点的增长**：表格、列表虚拟化（`uniform_list`）、拖拽是否进第二版。
- **插件 block 能否嵌其他 block 的内容**（例如显示某个终端的最后几行）：权限上可行，视图上需要一个 `block-preview(id)` 节点。
- **Go 版对 `kind: "plugin"` 的处理**：需先确认。
- **MCP 侧**：agent 能否调用插件导出的“动作”（`on-mcp`），还是只能改 `data`。

## 落地顺序

1. **兼容先行**：`BlockKind::Unknown` 原样保留不认识的 block，单独发版。
2. **宿主骨架**：`src/plugin/`（`host.rs` wasmtime 与线程，`view.rs` 视图树 → GPUI，`manifest.rs`），WIT 定义放 `wit/`。
3. **计数器 block**：只做 `col/row/text/button` + `on-event` + `data` 持久化，验证渲染、缩放、LOD、保存整条链路。
4. **`Change::Fields`** 与插件 block 的撤销。
5. **监控**：快照差分、事件合并、`subscribe` 过滤；示例插件监听终端增删。
6. **宿主 API** 接 MCP 工具层，写入并入用户操作的 `Group`。
7. 权限界面、`input` / `canvas` / `markdown` 节点、SDK 发布。

## 参考

- Zed 扩展：wasmtime + WIT，扩展 API 的版本化做法值得参考（`zed_extension_api`）。
- wasmtime Component Model、`wit-bindgen`。
- 本仓库：`deck/lifecycle.rs`（生命周期）、`mcp/tools.rs`（字段表与工具）、`deck/sharpen.rs`（过渡分类）、`markdown.rs`（勾选框的点击写回）。
