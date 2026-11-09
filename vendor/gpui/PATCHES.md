# BoundlessDeck 对 gpui 0.2.2 的补丁

本目录是 crates.io 上 gpui 0.2.2 的源码（去掉了 `examples/`），通过根目录 `Cargo.toml` 的
`[patch.crates-io]` 替换上游。改动处都以 `BoundlessDeck:` 注释开头，可用
`grep -rn "BoundlessDeck" vendor/gpui/src` 找到。升级上游时，取新版源码后把以下补丁重新打上。

## 1. 按字号缓存 CTFont（`src/platform/mac/text_system.rs`）

`layout_line` 原本每排一行、每个 run 都 `clone_with_font_size` 新建一个 CTFont，CoreText 随之重建该字体的度量缓存
（`TFont::InitASCIIDataCache`，可变字体还要算 HVAR 变化量），约占换字号时排版耗时的五分之一。
现在以 `(FontId, 字号)` 缓存（`sized_fonts`），超过 256 个时清空。结果与原来逐字节相同。

## 2. 同步提交前等上一帧异步提交完成（`src/platform/mac/metal_renderer.rs`）

`displayLayer:` 把图层切到 presentsWithTransaction 同步提交时，若上一帧异步提交的 drawable 还在路上，
它会被扣到下一次事务提交才显示；事务提交又要等主线程，主线程却阻塞在 `next_drawable` 等这个 drawable，
直到 CoreAnimation 的 1 秒超时，且这一帧不画。表现为缩放结束时画面卡住约 1 秒。
现在记住最后一个异步命令缓冲（`last_async`），切到同步提交前 `wait_until_completed`（实测 0–5 ms）。

## 3. 预排下一帧的文字（`src/text_system/line_layout.rs`、`src/text_system.rs`）

新增 `WindowTextSystem::prefetch_scaled(ratio)`：把上一帧排过的每一行（含自动换行段落的原始行）在后台线程
按 `ratio` 倍字号重新交给 CoreText 排版，放进 `Prefetch.ready`；之后 `layout_line` 未命中帧缓存时先查这里
（字号按 1/64 px 取整匹配，再精确缩放到请求的字号）。每次请求作废之前的请求；后台线程先等 16 ms，
连续请求时只有最后一次真正干活。

应用在照片式缩放的每一步调用它（`src/deck/photozoom.rs` 的 `photo_moved`），停手后的清晰帧因此大多不必在主线程排版。
