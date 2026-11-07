# BoundlessDeck-RS

BoundlessDeck（Go + MyGo 版，见上级目录）的 Rust 重写，界面用 GPUI（Zed 的 GPU UI 框架）原生绘制。

```sh
cargo run                      # 主页
cargo run -- <文件夹>          # 直接打开该文件夹的 deck
cargo test                     # 模型、持久化、编辑器外的纯逻辑测试
```

试运行时可用 `BOUNDLESSDECK_DATA_DIR`（recents、settings）与 `BOUNDLESSDECK_ACTIVE_FILE`（MCP active-deck 注册表）指到临时目录，避免改动真实数据。

本机没有 Xcode 的 `metal` 编译器，所以 gpui 开启了 `runtime_shaders` 特性（运行时编译着色器）。

## 与 Go 版的兼容

`.boundlessdeck/deck.json` 格式与 Go 版一致（字段名、省略空值、终端历史的 base64、时间格式），两版可以打开同一个文件夹。未实现类型的 block 数据原样保留。

## 第一阶段已完成

- 画布：平移（双指/右键/中键拖）、⌘+滚轮缩放并吸附到缩放档位、⌘0 / ⌘= / ⌘-、框选、Shift+点击、⌘D、⌘Z / ⇧⌘Z、双击空白新建 QuickLook、顶部空白拖窗口
- block：胶带拖动、标题栏拖动、点标题改名、右下角等比缩放、LOD（< 30% 只画框和标题）、拖进块堆
- 文件预览：PDF（CoreGraphics 渲染、翻页）、Word（.docx 解析为 Markdown，.doc 经 textutil）、Markdown 中的图片
- QuickLook（便签、Markdown、到期淡出与消失、剩余时间菜单）、笔记（Markdown 文件、自动保存、外部修改回读、改名即改文件名）、文件（文本带行号、Markdown、图片）、终端（alacritty_terminal；每次打开 deck 都在其文件夹中启动新的 shell，不恢复历史）
- 块堆：钱包式卡片堆叠，点击后悬停推出、上下边缘或滚轮滚动、空格长预览（←/→ 翻看）、拖动卡片标题取出（放到画布 / 别的块堆 / 原处），一次撤销还原；卡片推出与滚动用带回弹的弹簧动画，长预览从卡片处展开、关闭时缩回，其余卡片让开
- 图表：柱状、条形、折线、面积、饼图，圆整刻度（万/亿）、图例、悬停提示；标题栏切换类型、表格按钮在旁边实时编辑数据
- 浏览器：wry（WKWebView）原生网页嵌在 block 里，后退/前进/刷新、地址栏（网址或搜索词）、标题与地址随页面同步、多标签页（新窗口链接在当前标签后新开一个标签，每个标签独立网页，deck.json 用 `tabs` 与 `tab` 保存，`url` 始终是打开的标签，Go 版只看到这一页）、非网页链接交给系统；被其他 block/工具栏/浮层遮住或缩放手势进行中时隐藏原生网页、在原位显示 WebKit 拍的快照（随 block 缩放，可被其他 block 正常遮挡），缩放停下后按画布比例设置页面缩放
- MCP：打开 deck 即在 `.boundlessdeck/` 起 Unix socket 端点（带 token），写 `mcp.json`、`block-schema.json` 并登记为 active deck；17 个工具（读写 block、批量改、删除、移动、对齐/分布/排列、选择集、撤销重做、保存）与 3 个资源；`boundlessdeck mcp-stdio [--workspace DIR]` 为 stdio 桥，无 deck 时也能握手并说明原因。文件格式与 Go 版一致，两版的桥可以互通
- 设置窗口（⌘,）：为 Claude Code / Claude 桌面版 / Cursor / VS Code 安装或卸载全局 MCP（只改我们这一项，保留其他 server 与键顺序，读不懂的配置不碰）、当前图版、项目级配置开关与移除
- 键盘快捷键窗口（⌘/）：菜单、按键绑定与此窗口出自同一张命令表，外加画布手势
- 主页：透视倾斜的点阵画布与其上的卡片（手工投影，近处淡出代替模糊）、应用图标里的机器人标志、「创意/创造/创作」轮换文字、最近列表的 deck 标记
- 触控板捏合缩放：GPUI 0.2.2 不转发 magnify 事件，改用 AppKit 本地事件监视器截获，交给画布以指针为中心缩放
- 主页：最近的 deck、打开文件夹、拖入文件夹、移除记录；菜单栏与快捷键

## 与 Go 版的差异

功能已全部移植，以下是实现上的不同或尚未细化之处：

- 缩放手势中将文字整形、折行宽度、字形/SVG 光栅化、终端字号与 PDF 渲染锁定在离散缩放档位（`lay` / `content_scale`），松手停顿 150ms 后平滑吸附到最近缩放档位，复用 GPUI 的 `LineLayoutCache` 与 `MetalAtlas`。
- 块堆取出卡片是拖动卡片标题出堆，而不是长按右上角。
- 主页场景的近景模糊用淡出代替（GPUI 无模糊滤镜）；轮播文字切换时用拖影（叠几层半透明副本）近似运动模糊，不是真正的高斯模糊；Deck 视图里的 LOD 切换没有模糊过渡。
- 终端暂不支持鼠标选择文字与复制、鼠标上报；文本编辑器没有自己的撤销。
- 透明图片下没有棋盘格。
- 浏览器的遮挡判断里，工具栏区域按固定尺寸估算。
