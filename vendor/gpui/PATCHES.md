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

## 3. 文字按字号档位排版，缩放不再重排（`src/text_system/line_layout.rs`）

`LineLayoutCache::layout_line` 不再按请求的精确字号调用 CoreText，而是按字号所在的档位排版：
每二倍区间 4 档（`STEPS_PER_OCTAVE`），档位与请求字号最多差约 9%。同一段文字在同一档位只排一次，
结果跨帧缓存在 `shapes`（上限 `MAX_SHAPES` 行，满了清空），再把字形位置、行宽、上下沿按比例缩放到请求的字号
（`scaled_to`）。字形仍按请求的字号光栅化，静止时清晰；换行由缩放后的位置计算，所以缩放时断行不变。
档位与实际字号的差别只体现在可变字体的光学尺寸（极细微的字距）。`force_width` 的行（终端等宽排列）不走档位。
