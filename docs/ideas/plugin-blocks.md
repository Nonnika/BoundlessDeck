# 插件 block（后期想法）

> 状态：想法，未排期、未实现。记录于 2026-11-07。

让用户用 Rust 编写插件，为画布添加新的 block 种类。

## 结论：用 WebAssembly 插件

插件用 Rust 编写，编译为 wasm（`wasm32-wasip2`），BoundlessDeck 通过 wasmtime 加载。宿主仍用 GPUI 绘制；插件只描述“画什么”和“怎么响应”。

### 为什么不用另外两种

| 方案 | 问题 |
|---|---|
| Rust 动态库（dylib + `libloading`） | Rust 没有稳定 ABI：插件必须与应用使用同一编译器版本、同一版 GPUI，任一方升级即崩溃。GPUI 的元素和 `Context` 也不能跨库边界传递。 |
| 编进应用（插件 crate 实现 trait，开 Cargo feature） | 插件可直接使用 GPUI，但每加一个插件都要重新编译整个应用。适合内置 block，不算面向用户的插件系统。 |

wasm 的好处：
- 接口稳定，插件不随应用版本失效。
- 在沙盒里运行：插件崩溃或死循环不会拖垮应用；能访问的文件和网络由宿主授权。
- Zed 的扩展采用同样做法（Rust → wasm，接口用 WIT 定义）。

## 设计草案

### 1. 用 WIT 定义接口

插件导出：
- `render(state, size, scale) -> 元素树`：声明式的 `box / text / image / button / input / path…`
- `on-event(state, event) -> 新 state`：点击、键盘、滚轮、定时器
- `on-mcp(field, value)`（可选）：让 agent 也能修改该 block

宿主提供给插件的能力（按 `plugin.toml` 中声明的权限授予）：
- 读写 deck 文件夹内的文件
- `http-get`
- 定时器
- 日志

### 2. 宿主侧改动

- **模型**：`BlockKind`（`model.rs`）目前是封闭枚举，`deck.rs` 等约 10 个文件按它分支。新增 `BlockKind::Plugin`；`Block` 增加 `plugin`（插件 ID）与 `data`（插件自己的 JSON 状态，存入 deck.json）两个字段。
- **渲染**：把插件返回的元素树翻译为 GPUI 元素，坐标与字号乘 `cam.s`。这样它与其他 block 一样遵守真实缩放（文字清晰）和 LOD（`cam.s < LOD_SCALE` 时只画框和标题）。
- **线程**：插件调用在后台执行（`cx.background_spawn`），结果经 `this.update` 回到 UI 线程，与现有加载流程一致。
- **加载**：插件位于 `~/Library/Application Support/BoundlessDeck/plugins/<id>/`，含 `plugin.wasm` 与 `plugin.toml`（名称、图标、默认大小、所需权限）。工具栏与菜单从此处列出可添加的插件 block。
- **撤销 / MCP**：
  - 插件 block 仍只经 `Model::add_block` / `Model::remove_block` 进出画布。
  - `data` 的修改记为一条 `Change`。
  - MCP 字段表（`mcp/tools.rs`）加入 `plugin` 与 `data`。

### 3. 插件作者的 SDK

提供一个 crate（暂名 `boundlessdeck-plugin`），封装 WIT 生成的绑定：

```rust
struct Counter;

impl Block for Counter {
    type State = u32;
    fn render(n: &u32, _: Size) -> El {
        column([text(format!("{n}")), button("加一", Msg::Inc)])
    }
    fn on_event(n: &mut u32, _: Msg) {
        *n += 1
    }
}

export_block!(Counter);
```

## 待决问题

- **元素词汇的范围**：先支持文本、图片、按钮、输入框、矢量路径、滚动区，可覆盖大部分需求。插件拿不到 GPUI 的全部能力，这是换取稳定与沙盒的代价。
- **与 Go 版 deck.json 的兼容**：需先确认 Go 版遇到未知 `kind` 时的行为。至少要做到 Go 版打开时原样保留该 block 及其 `data`，不丢数据。
- **性能**：每帧调用 wasm 渲染有开销。只在 state 或尺寸变化时重新 `render`，其余帧使用缓存的元素树。

## 第一步（原型）

只支持文本、按钮、矢量路径与 `on-event`，用一个“计数器”插件验证整条链路：
- 改 `model.rs`（`BlockKind::Plugin`、`plugin` / `data` 字段）
- 新增 `src/plugin/`（wasmtime 宿主、元素树 → GPUI 翻译）
- 新增 SDK crate
