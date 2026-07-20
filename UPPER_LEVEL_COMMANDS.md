# 上层集成的绘制命令对照表

本文是实现 `skia` **上层集成层**时使用的能力对照表，不是让上游调用方
调用方直接依赖 `Canvas`、`DisplayListBuilder` 或 `GpuCommandEncoder` 的 API 文档。
这些类型记录的是 Skia 下层目前能执行或编码的绘制能力；上层集成层应根据本表把上游
请求转换为合适的下层调用。

## 调用方向与职责

```text
上游调用方
  └─ 描述“画什么”、页面/资源数据、目标与渲染选项
       └─ Skia 上层集成层
            ├─ 校验请求、管理资源生命周期与缓存
            ├─ 选择 CPU、GPU 或平台目标
            └─ 调用下层 Canvas、DisplayList 或 GPU encoder
                 └─ core / image / CPU / GPU / Metal 执行
```

上游不应直接创建 `Canvas`、`Surface`、`DisplayListBuilder`、`GpuCommandEncoder` 或选择
执行后端。它只提供渲染意图、目标描述与必要的源数据；Skia 上层负责将这些内容传递给
下层。入口负责建立渲染上下文和目标，随后由内部 renderer 与 device 完成实际绘制。

当前仓库中的 `Canvas`、`DisplayList` 和 GPU encoder 是下层能力，尚不是统一的上层
`RenderRequest` / `RenderTarget` 接口。本文用于约束该集成层未来应覆盖哪些能力，不能
据此把下层类型泄露给上游调用方。

### 位图输入

上层收到 PNG、JPEG 或 WebP 的**不可信编码字节**时，应先通过 facade 导出的
`ImageCodec::decode`（或带调用方配额的 `decode_with_limits`）解码为 `ImageAsset`，再登记
其中的 `Image` 并绘制。`ImageAsset` 同时保留可写回的 EXIF；`Image` 记录 RGBA8 像素及其
`ColorSpace`，ICC profile 仅在它与像素颜色空间一致时写回。`CodecLimits` 必须由处理不可信
输入的上层按资源预算收紧；默认值只是通用安全上限。

输出图片时，调用 `ImageCodec::encode(&asset, &EncodeOptions::new(...))`，或使用
`encode_to` 写入受限流。PNG 通过 `PngOptions` 控制 Deflate 和 row filter；JPEG 需要显式
`JpegOptions`，支持 4:4:4 / 4:2:2 / 4:2:0、baseline / progressive 和
`JpegOptimization` 的 Fast / Balanced / Smallest 稳定策略；透明像素必须选择
`JpegAlphaHandling::Flatten`，否则编码失败。`JpegOptions::web_v1()` 是 quality 85、4:2:0、
progressive、Balanced 的版本化网页输出策略。WebP 当前只能使用
`WebPOptions::lossless_v1()`，请求有损 WebP 会明确返回不支持，不会降级。可选 EXIF 默认
剥离，只有 `MetadataPolicy::Preserve` 才会写入；与 `Image` 颜色解释一致的 ICC 会写回。
`EncodeLimits` 应按输出预算收紧。

格式实现保持在 `skia-codec` 内部，并平铺为同级私有模块 `codec/src/png.rs`、
`codec/src/jpeg.rs`、`codec/src/webp.rs`。PNG 和 WebP 经 `image` 分别使用纯 Rust `png` 与
`image-webp`；JPEG 解码经 `image` 使用 `zune-jpeg`，高级 JPEG 编码直接调用纯 Rust
`mozjpeg-rs`。上层只依赖上述稳定策略 API，不接触或选择这些实现 crate。`mozjpeg-rs` 的
默认 feature 已关闭，因此产品依赖中不引入 `mozjpeg-sys` 或 C mozjpeg。

`ImageCodec` 是唯一的 PNG/JPEG/WebP 文件入口；旧的 `EncodedImageFormat` 和裸 `Image` encode
接口已移除。

### 字体与文字输入

上层持有 TrueType、OpenType 或字体集合的编码字节时，通过
`FontFace::from_bytes(FontId::new(...), bytes)` 加载单字体文件的第 0 个 face；字体集合或
不可信输入使用 `FontFace::from_bytes_with_limits`，显式提供 face index 和 `FontLimits`。
`FontId` 由上层资源管理器稳定分配，不能使用平台字体句柄。

加载后可通过 `FontFace::family_name()` 读取优先的 OpenType typographic family（缺失时回退
legacy family），并通过 `style()` 读取 1–1000 weight、九级 `FontWidth` 和
`FontSlant`。需要选择字体时，使用 `FontStyle::new(weight, width, slant)` 构造请求，再调用
`FontCollection::match_face`；多 family 请求使用 `match_face_for_families`，它严格按调用方
family 顺序选择第一个存在的 family。family 名采用 ASCII 大小写不敏感比较。

同一 family 内采用 CSS-like 固定顺序：先 width（normal 及更窄请求优先向窄侧搜索，更宽请求
优先向宽侧搜索），再按 Normal/Italic/Oblique 的相邻偏好选 slant，最后按 CSS 的
400/500 特殊规则选择 weight；完全同分时保留 face 添加顺序。匹配本身不检查字符覆盖。
上层应把匹配结果 clone 到实际绘制使用的有界 `FontCollection` 首位，再按语言/脚本策略添加
fallback face；`FontFace` clone 只共享不可变字体字节。

只有一个已选定字体和单方向 UTF-8 segment 时，调用 `face.shape(text, font_size_bits)`；需要
显式方向时使用 `shape_with_direction`。这些方法通过 OpenType/AAT shaping 生成带 UTF-8
cluster、字形位置和 advance 的 `GlyphRun`。

需要跨字体 fallback 或混合 LTR/RTL 段落时，创建有界 `FontCollection`，按优先级调用
`add_face`，再调用 `shape_paragraph`；需要强制段落基方向时使用
`shape_paragraph_with_direction`。collection 在 extended grapheme cluster 边界选择第一个完整
覆盖该 grapheme 的字体，通过 Unicode bidi level 分段，并返回视觉从左到右排列的
`ShapedParagraph`。每个 `ShapedRun` 保留原始 UTF-8 范围、全局 cluster、方向和 Q16.16 横向
origin。

CPU 即时绘制使用
`Canvas::draw_shaped_paragraph(&paragraph, &collection, baseline_origin, paint)`。它逐个应用 run
origin，且成功或失败都会恢复 canvas 状态。单 run 仍可调用
`Canvas::draw_glyph_run(&run, &face, paint)`。DisplayList 当前没有专用 paragraph 命令；上层
录制时需登记每个 `GlyphRun`，并按 `ShapedRun::origin_x_bits` 录制相应 save/transform/draw/
restore 命令。

多行文本先用 `FontFace::metrics(font_size_bits)` 获取单字体 Q16.16 ascent、descent 和 line
gap；通常直接创建 `TextLayoutOptions::new(max_width_bits)`，再调用
`collection.layout_text(text, font_size_bits, options)`。布局器使用 Unicode line-break
opportunity 做贪心换行，每一行重新执行 bidi/fallback/shaping，避免在连字或上下文 shaping
结果中间直接切 glyph。没有合法软断点且首个单词超宽时，会在 extended grapheme 边界强制
断行；显式 CR/LF/CRLF/NEL/LS/PS 产生 hard break，尾随换行会保留空白末行。

`TextLayoutOptions::with_limits` 可限制总行数和候选 shaping 次数；`TextLayout` 给出总
width/height，每个 `ShapedLine` 给出全局 UTF-8 范围、baseline Y、metrics 和 hard-break
标记。CPU 用 `Canvas::draw_text_layout(&layout, &collection, top_left, paint)` 一次绘制所有
非空行。

需要语言词典分词或断字时，由上层实现
`TextBreakProvider::opportunities(word, language)`，返回相对当前 Unicode word 的
`TextWordBreak` 列表，再调用
`layout_text_with_break_provider(text, size, options, bcp47_language, &provider)`。
`TextWordBreakKind::Soft` 只增加无 glyph 换行点，适合复杂上下文字系的词典分词；
`Hyphenated` 会在采用断点时生成可见连字符。provider 不应返回词首、词尾、非 UTF-8
boundary 或 extended-grapheme 内部位置；布局器会再次校验、排序和去重，非法结果返回
`InvalidWordBreak`。language 使用非空、连字符分段的 BCP 47-style ASCII tag，结构非法时
返回 `InvalidLanguage`。

词典断点和 UAX #14 候选会一起参与贪心宽度选择；未采用的断点不会产生字符。采用
`Hyphenated` 断点时，布局器优先插入 U+2010 HYPHEN，字体不覆盖时回退 ASCII `-`，并把它
放在逻辑断点所属 bidi run 的正确视觉侧。synthetic run 的 `source_start == source_end`，
glyph cluster 等于原文断点，`ShapedLine::hyphenated()` 可区分这类行。provider 返回的总
候选数与 `max_shaping_attempts` 共享工作上限；核心不捆绑具体语言词典，上层负责词典版本、
缓存和语言回退。

横向排版通过 `TextLayoutOptions::with_alignment` 选择 `TextAlignment::Start`、`End`、
`Left`、`Center`、`Right` 或 `Justify`。默认 `Start` 会按每行段落基方向选择物理左右边：
LTR 从左开始，RTL 从右开始；`Left` / `Right` 始终使用物理边。`ShapedLine::offset_x_bits`
是相对 text-block origin 的最终横向位置，`advance_x_bits` 是 justification 后的行宽，
`TextLayout::container_width_bits` 保留调用方给出的容器宽度。

`Justify` 只扩展行内、非首尾的 ASCII 空格，并通过 `ShapedRun::glyph_offsets_x_bits` 保存
逐 glyph 的 Q16.16 位移，不修改 shaping cluster 或 bidi run 顺序。默认不处理段落末行；
确实需要时显式调用 `with_justify_last_line(true)`。没有可扩展空格的行回退为逻辑
`Start`，不会伪造字符间距。DisplayList 展开布局时除了 run origin，还必须应用 line
offset 和每个 glyph 的额外 offset；CPU `draw_text_layout` 已自动完成这些步骤。

`FontFace` 内部使用纯 Rust `rustybuzz` 完成 shaping，并通过其 `ttf-parser` 解析矢量轮廓；
字体字节由 face 自身不可变持有。轮廓的字体坐标会转换为 canvas 向下为正的坐标，再复用普通
path fill 管线。空格等没有矢量轮廓的字形可以参与 shaping 和 advance，但绘制时不产生路径。

当前 text 层已负责**单段 shaping、单段落 bidi、按序 fallback、字体 metrics、通用 Unicode
换行、可插拔词典分词/断字、OpenType family/style 元数据和匹配、逻辑/物理对齐、ASCII 空格
justification 和轮廓解析**，但不负责平台字体发现、generic family 映射、variable axis
实例化、语言偏好、内置词典/断字算法、非 ASCII justification 或文本装饰。
`shape_paragraph` 只接受一个未换行段落；多段内容应使用 `layout_text`。缺少覆盖字体会返回
`MissingGlyph`。当前 Unicode line-break 实现把 SA 复杂上下文字系按普通字母处理；泰文、
老挝文、高棉文和缅甸文需要上层通过 `TextBreakProvider` 接入合适的 `Soft` 词典边界。

## 先看结论

| 能力 | CPU `Canvas`（即时执行） | `DisplayListBuilder`（录制后由 CPU 回放） | `GpuCommandEncoder`（录制后提交后端） |
| --- | --- | --- | --- |
| 清屏 | `clear` | `clear` | `clear` |
| 保存/恢复状态 | `save` / `restore` | `save` / `restore` | `save` / `restore` |
| 设置变换 | `set_transform`、`concat` | `set_transform`、`concat_transform` | `set_transform`、`concat_transform` |
| 矩形裁剪 | `clip_rect` | `clip_rect` | `clip_rect` |
| 填充矩形 | `fill_rect` | 无（用矩形 `Path` + `fill_path`） | `fill_rect` |
| 填充路径 | `fill_path` | `fill_path` | `fill_path` |
| 描边路径 | `stroke_path` | `stroke_path` | **无** |
| 绘制位图 | `draw_image` | `draw_image` | `draw_image` |
| 绘制文字 | `draw_glyph_run`、`draw_shaped_paragraph`、`draw_text_layout` | `draw_glyph_run`（paragraph/layout 需展开） | **无** |
| 当前实际硬件后端 | CPU 可用 | CPU 可用 | Metal 目前仅支持 `clear` |

因此，`Canvas` 是现阶段下层命令最完整、也是语义参考实现；`DisplayList` 适合由上层
缓存或跨线程传递同一组 CPU 绘制；GPU 命令集和 Metal 后端都还没有覆盖完整的 Canvas
能力。

## 通用约定

- 坐标使用固定点 `Scalar`（Q16.16）；通过 `Scalar::from_i32` 或
  `Scalar::from_ratio` 创建。计算溢出返回 `NumericOverflow`，不会静默截断。
- `Rect::new(left, top, right, bottom)` 必须是正面积矩形（`left < right` 且
  `top < bottom`），坐标系原点在左上。
- `Color` 是 straight-alpha 的 sRGBA8。`BlendMode` 覆盖 Porter-Duff、Plus、Modulate 及
  Multiply/Screen/Overlay 等高级混合；它描述**像素合成**，不是路径的 union/intersect 等
  几何布尔运算。
- 变换是仿射矩阵 `(a, b, c, d, e, f)`。`set_transform` 替换当前变换；Canvas 的
  `concat(next)`、DisplayList/GPU encoder 的 `concat_transform(next)` 均表示先应用当前
  变换、再应用 `next`。
- `save` 保存变换和裁剪，`restore` 恢复最近一层；没有匹配的 `save` 会返回
  `RestoreUnderflow`。Canvas 默认最多 256 层，可由 `SurfaceLimits` 收紧。
- `clear` 总是作用于整个目标，忽略当前变换和裁剪。

## 1. CPU Canvas：下层即时执行路径

调用顺序为：`Surface::new(...)` → `surface.canvas()` → 以下命令。`Canvas` 持有对
`Surface` 的可变借用，结束后可经 `Surface::pixels()` 读取紧密排列的 RGBA8 像素。

### 状态命令

| 命令 | 作用 | 现有边界 |
| --- | --- | --- |
| `clear(color)` | 用一个颜色覆盖整个目标。 | 忽略状态；无返回值。 |
| `save()` | 压入当前变换与裁剪。 | 受 `max_save_depth` 限制。 |
| `restore()` | 弹出并恢复最近状态。 | 空栈报错。 |
| `set_transform(transform)` | 替换后续绘制使用的变换。 | 不会与旧变换相乘。 |
| `concat(transform)` | 把变换追加到当前变换。 | 可能因固定点计算溢出失败。 |
| `clip_rect(ClipRect::new(rect))` | 用变换后的矩形和当前裁剪求交。 | 当前变换含旋转或错切时返回 `UnsupportedTransform`；仅矩形裁剪。 |

### 图形命令

| 命令 | 作用 | 已确定的行为/边界 |
| --- | --- | --- |
| `fill_rect(rect, paint)` | 填充变换后的矩形。 | 通过通用路径光栅化，允许旋转/错切。 |
| `fill_path(path, rule, paint)` | 按 `EvenOdd` 或 `NonZero` 填充路径。 | 二、三次贝塞尔以固定步数展平；开放轮廓在填充时会隐式闭合。 |
| `stroke_path(path, width, paint)` | 描边路径。 | 宽度必须为正；固定步数曲线展平；只有**圆头、圆角**，没有 butt/square cap、miter/bevel join、虚线或描边对齐选项。 |
| `draw_image(image, destination, opacity, blend_mode)` | 绘制 RGBA8 图片到目标矩形。 | 最近邻采样；`opacity` 只乘源 alpha；旋转或错切变换被拒绝，尚无逆变换采样和滤镜。 |
| `draw_glyph_run(run, provider, paint)` | 根据字形轮廓填充一段已整形文字。 | `FontFace` 提供单字体 shaping；`FontCollection` 提供 bidi/fallback 后的多个 run；字体发现仍由上层负责；缺失轮廓会跳过。 |
| `draw_shaped_paragraph(paragraph, provider, origin, paint)` | 在同一 baseline origin 绘制视觉顺序的所有字体 run。 | `FontCollection` 同时充当多字体轮廓 provider；方法内部隔离每个 run 的状态。 |
| `draw_text_layout(layout, provider, origin, paint)` | 从 top-left origin 绘制所有非空行。 | baseline、空行、行高、横向对齐和 justification 位移由 `TextLayout` 固化；仍复用 paragraph/run/path 管线。 |

## 2. DisplayList：下层可移植 CPU 命令表

`DisplayListBuilder` 把资源和绘制命令录制为不可变 `DisplayList`，再通过
`Surface::execute_display_list(&list, &glyph_provider)` 回放。它的命令枚举为：

| `DrawCommand` / Builder 方法 | 参数 | 说明 |
| --- | --- | --- |
| `Clear` / `clear` | `Color` | 清屏。 |
| `Save` / `save` | 无 | 保存状态。 |
| `Restore` / `restore` | 无 | 恢复状态。 |
| `ClipRect` / `clip_rect` | `Rect` | 矩形裁剪。 |
| `SetTransform` / `set_transform` | `Transform` | 替换变换。 |
| `ConcatTransform` / `concat_transform` | `Transform` | 叠加变换。 |
| `FillPath` / `fill_path` | `PathId`、`FillRule`、`Paint` | 填充已登记的路径。 |
| `StrokePath` / `stroke_path` | `PathId`、正 `Scalar` 宽度、`Paint` | 描边已登记的路径。 |
| `DrawImage` / `draw_image` | `ImageId`、目标 `Rect`、`u8` opacity、`Paint` | 绘制已登记图片；使用 `paint.blend_mode()`。 |
| `DrawGlyphRun` / `draw_glyph_run` | `GlyphRunId`、`Paint` | 绘制已登记的整形字形序列。 |

资源须先经 `add_path`、`add_image` 或 `add_glyph_run` 登记，取得仅在该列表中有效的 ID；
`finish()` 后发布列表。构建器的 `max_items` 同时限制**命令数及每一种资源数**。

### 与 Canvas 的差异

- 没有 `fill_rect`：需要用 `PathBuilder::add_rect` 建路径后调用 `fill_path`。
- 命令本身不携带“当时状态”的快照；回放时按命令顺序维护状态。因此保存/恢复顺序是
  列表语义的一部分。
- 回放使用 Canvas，所以最终约束与 CPU Canvas 一致，包括图片轴对齐限制、描边样式和
  文字轮廓解析要求。

## 3. GPU：下层编码与提交路径

`GpuCommandEncoder` 是另一套后端中立的命令表，不是 `DisplayList` 的执行器。流程为：
创建 encoder → 登记资源 → 录制命令 → `finish()` → 由实现 `GpuBackend` 的后端
`submit` 到 `GpuSurfaceDescriptor` 指定大小的表面。

| `GpuCommand` / Encoder 方法 | 参数 | 说明 |
| --- | --- | --- |
| `Clear` / `clear` | `Color` | 清空完整目标，不受裁剪影响。 |
| 状态 / `save`、`restore`、`set_transform`、`concat_transform`、`clip_rect` | 同名状态参数 | 这些方法修改 encoder 状态；每个绘制命令会记录当时的变换和 target-space scissor 快照。 |
| `FillRect` / `fill_rect` | `Rect`、`Paint` | 填充矩形。 |
| `FillPath` / `fill_path` | `GpuPathId`、`FillRule`、`Paint` | 填充已登记路径。 |
| `DrawImage` / `draw_image` | `GpuImageId`、目标 `Rect`、`u8` opacity、`BlendMode` | 绘制已登记 RGBA8 图片。 |

GPU encoder 也要求先调用 `add_path` / `add_image`。`GpuCommandLimits` 可分别限制命令、路径、
图片和状态栈深度。裁剪仍只接受轴对齐变换；裁剪为空时，后续绘制命令不会被录制。

### GPU 当前缺口

- GPU 命令层没有 `stroke_path`、`draw_glyph_run`，也没有专用的渐变、滤镜、图层或
  复杂裁剪命令。
- `SoftwareGpuBackend` 能用 CPU Canvas 回放上述 GPU 命令，主要用于一致性测试，并不是真正的
  硬件 GPU 实现。
- `MetalBackend` 当前会真实创建纹理并执行 `Clear`；遇到 `FillRect`、`FillPath` 或
  `DrawImage` 会返回 `UnsupportedCommand`。即 Metal 的着色器管线虽已有准备，硬件绘制命令尚未
  落地。

## 4. 路径构造能力（为下层绘制准备资源）

路径不是 `Canvas` 状态命令，但它决定 `fill_path` 和 `stroke_path` 可以表达哪些图形。使用
`PathBuilder::new(max_verbs)` 创建，并以 `finish()` 发布不可变 `Path`。

| 分组 | 方法 | 说明 |
| --- | --- | --- |
| 基本轮廓 | `move_to`、`line_to`、`quad_to`、`conic_to`、`cubic_to`、`close` | 直线、二次/有理二次/三次贝塞尔；除 `move_to` 外必须有活跃轮廓。 |
| 基本形状 | `add_rect`、`add_oval`、`add_circle`、`add_round_rect` | oval/圆角使用确定性的三次贝塞尔近似；圆半径必须正，圆角半径不得为负且会夹到矩形半宽/半高。 |
| 多边形 | `add_polygon` | 接受开放或闭合多边形；开放至少两个点，闭合至少三个点。 |
| 椭圆弧 | `add_arc` | 从 `ArcStart` 开始、按 `ArcDirection` 画 1–4 个 90° 段。 |
| 任意角度弧 | `add_arc_degrees`、`arc_to` | `Angle` 使用顺时针 canvas 度数；扫角不能为 0，绝对值不能超过一整圈；最多拆成四段三次贝塞尔。`arc_to` 会在需要时先连一条直线到弧起点。 |
| 旋转椭圆弧 | `add_rotated_arc_degrees`、`arc_to_rotated` | 在椭圆中心旋转后输出三次贝塞尔段；参数仍使用确定性 Q16.16 角度。 |
| 组合/查询 | `append_path`、`Path::reversed`、`Path::transformed`、`Path::bounds`、`Path::tight_bounds` | 支持追加、反向、生成变换副本、控制点保守边界和多项式贝塞尔 extrema-aware 保守边界。 |

## 用这份表排查不足

优先确认目标调用路径属于哪一层；不要把“CPU 已可画”误判为“DisplayList 或 Metal 已可画”。
当前最明显的不对齐项是：

1. `DisplayList` 缺少 `fill_rect` 的直接命令；
2. GPU 命令层缺少描边与文字；
3. Metal 尚未实现任何非清屏命令；
4. 裁剪仍只有矩形，图片仍是 RGBA8；
5. 图片不支持非轴对齐变换/过滤，描边样式也只有圆头圆角；
6. 文本层已有内存字体解析、family/style 匹配、单段落 bidi、跨字体 fallback、metrics、
   通用换行、可插拔词典断点、hyphenation、对齐与基础 justification，但仍没有系统字体发现、
   内置语言词典、variable axis、非 ASCII justification、装饰与完整排版；
7. 路径的几何布尔运算、stroke-to-path 和 path effects 尚未暴露；它们不能由像素混合模式替代。

源码入口：Geometry 在 `geometry/src/lib.rs`，Path 在 `path/src/lib.rs`，CPU Canvas 在
`cpu/src/canvas.rs`，字体加载与 shaping 在 `text/src/font.rs`，DisplayList 在
`core/src/display_list.rs`，GPU 命令层在
`gpu/src/lib.rs`，Metal 后端在 `gpu/metal/src/lib.rs`。

## Rust 工具链维护

本仓库使用 `rustup` 管理 Cargo 与 Rust 工具链。根目录的
`rust-toolchain.toml` 会让 `cargo` / `rustc` 自动选择 `stable`，并安装
`clippy` 和 `rustfmt`；`Cargo.toml` 的 `rust-version = "1.89"` 是本仓库当前的
最低支持版本（由纯 Rust `mozjpeg-rs` 要求）。更新 stable 工具链（以及 Cargo）后，用以下
命令确认版本：

```sh
rustup update stable
cargo --version
rustc --version
```

工作区验证会构建全部内部 crate 与可选后端：

```sh
cargo test --workspace --all-features
```

只验证上层公开图片 codec 契约时，运行根 crate 的 facade 集成测试；该测试不直接引用
`skia-codec`、`image` 或具体格式实现 crate：

```sh
cargo test --test codec_api
```

只验证字体加载、UTF-8 shaping、轮廓解析和 CPU 文字绘制链路时，运行：

```sh
cargo test -p skia-text -p skia-core -p skia-cpu --tests
```
