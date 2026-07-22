# 上层集成的绘制命令对照表

本文是实现 `skia` **上层集成层**时使用的能力对照表，不是让上游调用方
调用方直接依赖 `Canvas`、`DisplayListBuilder` 或 `GpuCommandEncoder` 的 API 文档。
这些类型记录的是 Skia 下层目前能执行或编码的绘制能力；上层集成层应根据本表把上游
请求转换为合适的下层调用。

## 仓库布局

Rust workspace 位于 `skia-rs/`。除非另有说明，本文中的源码路径和 Cargo 命令均相对于
该目录；仓库根目录的脚本应从根目录执行。

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

`ImageCodec` 是唯一的 PNG/JPEG/WebP 文件入口；动画入口可解码 APNG 和 animated WebP。WebP
解码后保留为完整、已合成的 canvas frames；animated WebP 编码尚不可用，会明确返回
`UnsupportedEncodeOption`。旧的 `EncodedImageFormat` 和裸 `Image` encode 接口已移除。

### 字体与文字输入

需要使用平台字体时，调用
`discover_system_fonts(&additional_roots, SystemFontDiscoveryLimits::default())`。独立的
`skia-system-fonts` adapter 会按平台枚举系统与用户字体目录，识别 TTF/OTF/TTC/OTC，按路径和
face index 生成稳定 `FontId`，并保留 family/style 元数据而不把目录策略放进 `skia-text`。
`SystemFontCatalog::match_generic` 解析 serif/sans-serif/monospace/system-ui/cursive/fantasy/
emoji/math，`match_language` 先尝试中日韩、阿拉伯、希伯来、天城文和泰文字体偏好，再回退
generic family。用 `SystemFontRecord::load` 只加载选中的 face；需要完整 fallback collection 时
调用 `load_collection`，同一 TTC/OTC 的多个 face 会共享一份不可变编码字节。

上层持有 TrueType、OpenType 或字体集合的编码字节时，通过
`FontFace::from_bytes(FontId::new(...), bytes)` 加载单字体文件的第 0 个 face；字体集合或
不可信输入使用 `FontFace::from_bytes_with_limits`，显式提供 face index 和 `FontLimits`。
已共享编码分配的 adapter 使用 `from_shared_bytes[_with_limits]`。`FontId` 由上层资源管理器或
system-font adapter 稳定分配，不能使用平台字体句柄。

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

variable font 通过 `FontFace::variation_axes()` 暴露 `fvar` axis；tag 使用
`FontTag::new(*b"wght")` 这类精确四字节值，min/default/max 均为 signed Q16.16。
上层选择实例坐标后，构造 `FontVariation::new(tag, value_bits)` 列表并调用
`instantiate_variations(new_font_id, &coordinates)`。返回的新 face 共享原始不可变字体字节，
但必须使用不同 `FontId`，以免不同 outline/advance 被资源解析器误认为同一字体。

coordinate 必须落在 axis 闭区间内，不能重复或引用未知 tag；违反时返回
`InvalidFontVariation`。未指定的 axis 保留字体默认值，显式指定 default 值不会保存在实例的
`variations()` 中。实例坐标会一致应用于 shaping、HVAR/MVAR metrics、underline/strikeout
metrics 和 gvar outline。core 只负责验证与执行坐标，不负责根据 CSS、设备或用户偏好选择
哪个实例。

需要控制 OpenType shaping feature 时，使用
`FontFeature::new(FontTag::new(*b"kern"), 0)` 这类全局 tag/value，再调用
`face.instantiate_features(new_font_id, &features)`。feature 实例同样共享字体字节但必须使用
新的 `FontId`；它保留已有 variable coordinates，并自动影响单 run shaping、fallback/bidi
paragraph、hyphenation 和 multiline layout 的所有 shaping 调用。`features()` 返回按 tag
规范化排序后的配置。

同一实例内 feature tag 不能重复，重复或复用原 `FontId` 返回 `InvalidFontFeature`；单实例
最多 256 项，超过时返回 `ResourceLimit`。字体不支持的 tag 仍是合法输入并由 shaping engine
忽略，这允许上层对 fallback 字体应用一致配置。当前 feature 是整 face 实例全局值；需要
按 source range 或 span 改变 `liga`、`kern`、stylistic set 等设置时，上层必须先分段并为各段
选择相应实例。

只有一个已选定字体和单方向 UTF-8 segment 时，调用 `face.shape(text, font_size_bits)`；需要
显式方向时使用 `shape_with_direction`。这些方法通过 OpenType/AAT shaping 生成带 UTF-8
cluster、字形位置和 advance 的 `GlyphRun`。

需要触发字体的语言相关 OpenType 行为（尤其 `locl`）时，单 face 使用
`shape_with_language` 或 `shape_with_direction_and_language`；collection 对应使用
`shape_paragraph_with_language`、`shape_paragraph_with_direction_and_language`、
`shape_styled_paragraph_with_language` 或
`shape_styled_paragraph_with_direction_and_language`。多行统一/样式布局分别使用
`layout_text_with_language` 和 `layout_styled_text_with_language`。language 必须是非空、
以连字符分段且每段只含 ASCII 字母数字的 BCP 47-style tag，否则返回 `InvalidLanguage`。
同一 language 会传给 fallback、bidi、逐行重塑、synthetic hyphen 与 ellipsis 的所有 shaping；
它不替上层选择语言字体或建立 fallback 顺序。

需要跨字体 fallback 或混合 LTR/RTL 段落时，创建有界 `FontCollection`，按优先级调用
`add_face`，再调用 `shape_paragraph`；需要强制段落基方向时使用
`shape_paragraph_with_direction`。collection 在 extended grapheme cluster 边界选择第一个完整
覆盖该 grapheme 的字体，通过 Unicode bidi level 分段，并返回视觉从左到右排列的
`ShapedParagraph`。每个 `ShapedRun` 保留原始 UTF-8 范围、全局 cluster、方向和 Q16.16 横向
origin。

同一未换行段落需要混合字体实例或字号时，为全文构造有序、连续、无重叠的
`TextStyleSpan::new(source_start, source_end, preferred_font_id, font_size_bits)` 列表，再调用
`shape_styled_paragraph`；强制基方向时使用 `shape_styled_paragraph_with_direction`。spans
必须完整覆盖 UTF-8 文本，边界必须落在 extended grapheme 之间，指定的 `FontId` 必须已经
加入 collection，否则返回 `InvalidTextStyleSpan`。

每个 grapheme 会先尝试 span 的 preferred face，再按 collection 原顺序 fallback；因此
fallback 不会因样式分段失效。不同字号会保存在各自 `GlyphRun`，paragraph metrics 取实际
使用 runs 的最大 ascent/descent/line gap。span 也可引用 variable/feature instance，从而
实现未换行段落内的 per-range axis 或 OpenType feature。

`TextStyleSpan::with_style_id(TextStyleId::new(...))` 可附加稳定、与 renderer 无关的样式标识，
`with_decoration(...)` 可覆盖该 span 的 underline/strike-through，
`with_decoration_style(...)` 可独立覆盖 Solid/Dashed/Dotted/Wavy 线型；不调用时分别使用默认样式 ID、
layout-wide decoration 和 layout-wide decoration style。颜色仍属于 `Paint`，因此 `skia-text` 只把 ID 保留到
`ShapedRun` 和 `TextDecorationSegment`，不会反向依赖 core paint。跨行 styled text 直接调用
`layout_styled_text(text, spans, options)`；spans 的覆盖、顺序、grapheme 边界和 FontId
约束与 styled paragraph 相同。每个候选行都会重新执行 bidi/fallback/shaping，不会直接切开
已有 glyph run。

CPU 即时绘制使用
`Canvas::draw_shaped_paragraph(&paragraph, &collection, baseline_origin, paint)`。它逐个应用 run
origin，且成功或失败都会恢复 canvas 状态。单 run 仍可调用
`Canvas::draw_glyph_run(&run, &face, paint)`。DisplayList 不增加 paragraph 专用执行命令，但
`DisplayListBuilder::draw_shaped_paragraph[_with_styles]` 会事务性展开为定位 run；任何 paint、坐标、
资源或命令预算失败都会回滚本次展开。

多行文本先用 `FontFace::metrics(font_size_bits)` 获取单字体 Q16.16 ascent、descent 和 line
gap；通常直接创建 `TextLayoutOptions::new(max_width_bits)`，再调用
`collection.layout_text(text, font_size_bits, options)`。布局器使用 Unicode line-break
opportunity 做贪心换行，每一行重新执行 bidi/fallback/shaping，避免在连字或上下文 shaping
结果中间直接切 glyph。没有合法软断点且首个单词超宽时，会在 extended grapheme 边界强制
断行；显式 CR/LF/CRLF/NEL/LS/PS 产生 hard break，尾随换行会保留空白末行。

`TextLayoutOptions::with_limits` 可限制总行数和候选 shaping 次数；`TextLayout` 给出总
width/height，每个 `ShapedLine` 给出全局 UTF-8 范围、baseline Y、metrics 和 hard-break
标记。CPU 用 `Canvas::draw_text_layout(&layout, &collection, top_left, paint)` 一次绘制所有
非空行。styled layout 的行 metrics 取该行实际 runs 的最大值；连续 hard break 产生的空行
使用其逻辑行首 span 的 preferred face 和字号，尾随换行空行使用最后一个 span。
需要 per-span paint 时改用 `draw_text_layout_with_styles` 或
`draw_shaped_paragraph_with_styles`，传入 `TextStyleId -> Option<Paint>` resolver；缺少任一 ID
会返回 `InvalidResource`，不会静默套用错误颜色。

需要语言词典分词或断字时，可直接创建缓存型 `BuiltinTextBreakProvider`，它通过
`hyphenation 0.8.4` 的 `embed_all` feature 嵌入 Knuth-Liang 词典，按 BCP 47 tag 延迟加载并
缓存；支持的 exact tag 使用对应词典，常用 region/variant tag 回退到基础语言，未知但合法的
tag 返回空候选。该依赖声明 Apache-2.0/MIT；上游 pattern 集合版本随锁文件固定。
产品需要自定义分词策略时也可自行实现
`TextBreakProvider::opportunities(word, language)`，返回相对当前 Unicode word 的
`TextWordBreak` 列表，再调用
`layout_text_with_break_provider(text, size, options, bcp47_language, &provider)`。
styled text 对应调用
`layout_styled_text_with_break_provider(text, spans, options, bcp47_language, &provider)`。
`TextWordBreakKind::Soft` 只增加无 glyph 换行点，适合复杂上下文字系的词典分词；
`Hyphenated` 会在采用断点时生成可见连字符。provider 不应返回词首、词尾、非 UTF-8
boundary 或 extended-grapheme 内部位置；布局器会再次校验、排序和去重，非法结果返回
`InvalidWordBreak`。language 使用非空、连字符分段的 BCP 47-style ASCII tag，结构非法时
返回 `InvalidLanguage`。这两个 break-provider 布局入口也会自动把同一个 language 传入
OpenType shaping，不需要再调用单独的 language layout API。

词典断点和 UAX #14 候选会一起参与贪心宽度选择；默认候选使用 Unicode 15 conformance
语料声明的 regex-number tailoring，完整数字表达式内部保持不可断，而脱离数字上下文的
标点/前后缀重新允许断行；LB30 的东亚宽开括号例外和 LB30b potential emoji 规则也包含在
候选生成中。未采用的断点不会产生字符。采用
`Hyphenated` 断点时，布局器优先插入 U+2010 HYPHEN，字体不覆盖时回退 ASCII `-`，并把它
放在逻辑断点所属 bidi run 的正确视觉侧。synthetic run 继承断点左侧实际 run 的字体实例、
字号和方向，再按 collection fallback 查找连字符；其 `source_start == source_end`，glyph
cluster 等于原文断点，`ShapedLine::hyphenated()` 可区分这类行。provider 返回的总候选数与
`max_shaping_attempts` 共享工作上限；内置 provider 负责词典版本、缓存和语言回退，自定义
provider 继续由上层自行管理。

横向排版通过 `TextLayoutOptions::with_alignment` 选择 `TextAlignment::Start`、`End`、
`Left`、`Center`、`Right` 或 `Justify`。默认 `Start` 会按每行段落基方向选择物理左右边：
LTR 从左开始，RTL 从右开始；`Left` / `Right` 始终使用物理边。`ShapedLine::offset_x_bits`
是相对 text-block origin 的最终横向位置，`advance_x_bits` 是 justification 后的行宽，
`TextLayout::container_width_bits` 保留调用方给出的容器宽度。

点到文本位置的命中使用 `TextLayout::hit_test_point(x_bits, y_bits)`；坐标是相对 layout
top-left 的 Q16.16 值，实际 Canvas 坐标应先减去绘制时传入的 origin。返回的
`TextHitResult` 包含最近的 zero-based line index 和 `TextPosition`。布局块外的 Y 会夹到最近
行，行外的 X 会夹到最近 caret stop。

反向查询使用 `caret_for_position(TextPosition::new(source_offset, affinity))`。
`TextAffinity::Upstream` 表示前一个 source cluster 之后，`Downstream` 表示后一个 cluster
之前；因此 soft wrap 同一个 UTF-8 offset 可分别定位上一行尾和下一行首，混合 bidi 边界也
可保留两个不同 X。`TextCaret` 返回 layout-local 的 X、top、bottom 和 line index，已包含
alignment、justification、styled 行高、空行和 synthetic hyphen。offset 不是 shaping
cluster boundary 时通常返回 `Ok(None)`；若字体的 OpenType GDEF `LigatureCaretList` 为该
ligature 给出与内部 extended-grapheme 边界一一对应的坐标，则这些边界也会返回 caret。
CaretValue Format 1 和 Format 3 的设计坐标受支持，Format 3 会应用 variable-font
VariationIndex delta；依赖轮廓 point index 的 Format 2 当前保持原子性。缺表、数量不匹配、
重复或落在 glyph advance 外的坐标都不会触发猜测或等分伪造。RTL run 会把逻辑 source 顺序
映射到反向的物理 caret 顺序。

逻辑选择区间使用 `TextLayout::selection_rects(source_start, source_end)` 转为一组
`TextSelectionRect`。两个 offset 必须是当前可见 layout 的 cluster edge 或上述 font-backed
ligature caret，逆序或无效边界返回 `InvalidLayout`，折叠区间返回空列表。结果按 line 和视觉顺序排列，给出
layout-local 的 left/top/right/bottom；跨行选择按行拆分，混合 bidi 中被未选 glyph 隔开的
区域会拆成多个 rect，相邻已选 cluster 之间的 letter/word/CJK justification 间距会包含在
合并 rect 内；ligature 内部区间按 GDEF caret 坐标产生局部矩形。synthetic hyphen/ellipsis
不消耗 source，因此不会产生选择矩形。

`TextLayoutOptions::with_limits(width, max_lines, max_shaping_attempts)` 的 line limit 默认仍是
all-or-error：超过时返回 `ResourceLimit`，不返回部分结果。明确允许截断时，再调用
`with_overflow(TextOverflow::Clip)` 或 `TextOverflow::Ellipsis`。`Clip` 保留前
`max_lines` 行并省略剩余 source；`Ellipsis` 会在最后一行按 extended grapheme 向前回退，
对每个候选 prefix 重新 shaping，直到 prefix 加 synthetic ellipsis 可放入 line width。
`TextLayout::truncated()` 表示确实省略过 source，`ShapedLine::ellipsized()` 只标记带
ellipsis 的最后一行；文本恰好用满 max lines 时两者保持 false。

ellipsis 优先使用 U+2026，缺字时回退三个 ASCII period；两者都不消耗 source bytes，其
synthetic run 的 `source_start == source_end ==` 可见 prefix 末端。非空行继承逻辑截断点
左侧实际 run 的字体实例、字号和 bidi 方向；空行使用逻辑行首 style。styled layout、字体
fallback、caret、alignment 和 decoration 因而继续生效。marker 自身比 line width 更宽时仍
保留 marker，和单个超宽 grapheme 的进度保证一致；两种 marker 都缺失时返回 `MissingGlyph`。
ellipsized line 不参与 justification。

字符与单词间距通过 `TextLayoutOptions::with_letter_spacing(signed_q16)` 和
`with_word_spacing(signed_q16)` 设置。letter spacing 只加在相邻 shaping cluster 之间，
同一 cluster 内的组合字符或连字 glyph 不会被拆开；word spacing 只加在行内可断 Unicode
space 后，NBSP、FIGURE SPACE 与 NARROW NBSP 不参与。两者会进入候选行宽，因此自动影响
wrap、ellipsis、alignment、hit testing 与 upstream/downstream caret。负值合法，但若最终
行 advance 变为负数则返回 `InvalidLayout`。这些 spacing 会与 justification 累加，且
`ShapedRun::glyph_offsets_x_bits` 同时保存两者产生的位移。

`Justify` 扩展行内、非首尾的可断 Unicode space separator：包括 ASCII SPACE、OGHAM SPACE
MARK、U+2000–U+2006、U+2008–U+200A、MEDIUM MATHEMATICAL SPACE 和 IDEOGRAPHIC SPACE；
NBSP、U+2007 FIGURE SPACE 与 NARROW NBSP 明确保持不可断、不可扩展。位移通过
`ShapedRun::glyph_offsets_x_bits` 按 glyph 保存，
不修改 shaping cluster 或 bidi run 顺序。默认不处理段落末行；确实需要时显式调用
`with_justify_last_line(true)`。`TextJustification::Auto` 在没有可扩展空格时，改在 CJK-CJK
或 CJK 与其他文字的安全相邻 shaping cluster 之间分配剩余宽度；
`with_justification(TextJustification::InterWord)` 可禁止此 fallback，`InterCharacter` 则显式
允许跨脚本的安全 cluster 边界。组合 mark、ligature 内部、空白、control 与标点边界不会被
拆开；跨 fallback/styled run 的相邻 cluster 仍可扩展。若没有合法 slot，才回退为逻辑
`Start`。
DisplayList 展开布局时除了 run origin，还必须应用 line offset 和每个 glyph 的额外 offset；
CPU `draw_text_layout` 已自动完成这些步骤。

整块 layout 需要下划线或删除线时，使用
`TextLayoutOptions::with_decoration(TextDecoration::Underline)`、
`StrikeThrough` 或 `UnderlineAndStrikeThrough`。统一字号 layout 的位置和粗细严格读取
collection 首个字体；styled layout 则读取每行逻辑行首 span 的 preferred face 和字号。
两者都使用 OpenType `post` underline metrics 与 `OS/2` strikeout metrics，并缩放为
Q16.16 canvas 坐标；`ShapedLine::underline_metrics()` 和
`strike_through_metrics()` 暴露最终值。这样同一条装饰线会跨越该行 fallback run 和空格
连续绘制，并跟随 line alignment/justification 后的 `offset_x_bits` 与
`advance_x_bits`。
线型通过 `TextLayoutOptions::with_decoration_style(TextDecorationStyle::Solid|Dashed|Dotted|Wavy)`
设置，默认保持 Solid；span 可单独覆盖线型而继续继承装饰种类，反之亦然。
span override 存在时，`ShapedLine::decoration_segments()` 以最终视觉顺序暴露各样式 ID 的
left/right、字体原生 metrics 和最终线型；CPU 会使用对应 span paint 绘制，并在同样式、同 metrics、同线型的
连续 fallback run 之间合并装饰区间。

请求的指标表缺失时，layout 返回 `MissingDecorationMetrics`，不会猜测平台相关默认值；字体
给出非正 thickness 时返回 `InvalidFontData`。空行不产生装饰，也不要求字体提供指标。
`text_decoration_rects` 在文本层按 Q16.16 指标统一生成有资源上限的 Solid/Dashed/Dotted/Wavy
矩形条带；CPU 和 GPU text adapter 共用节距、相位与波形规则。`Canvas::draw_text_layout` 在 glyph
之后用解析出的 span `Paint` 绘制这些条带。`DisplayListBuilder::draw_text_layout[_with_styles]`
会使用相同几何，把 glyph 录成定位 run、把装饰录成 `FillRect`，同时保留事务性失败语义。

`FontFace` 内部使用纯 Rust `rustybuzz` 完成 shaping，并通过其 `ttf-parser` 解析矢量轮廓；
字体字节由 face 自身不可变持有。轮廓的字体坐标会转换为 canvas 向下为正的坐标，再复用普通
path fill 管线。空格等没有矢量轮廓的字形可以参与 shaping 和 advance，但绘制时不产生路径。

需要小字号像素对齐或彩色 emoji 时，上层可对已选定的 glyph 调用
`FontFace::rasterize_glyph(glyph, font_size_bits)`。该纯 Rust 路径由 `swash` 执行：它优先采用
COLR/CPAL 分层颜色轮廓和内嵌彩色 bitmap，再回退到应用嵌入 hint 的 Alpha8 轮廓 bitmap。
返回的 `GlyphBitmap` 相对 glyph baseline 定位；在 canvas 坐标中绘制时，X 加 `left`，Y 减
`top`。缓存键至少包含 `font`、`glyph`、`font_size_bits` 和 `format`；Alpha8 必须使用调用方
`Paint` 的颜色混合，RGBA8 保留字体提供的颜色。当前 CPU `Canvas` 的通用文字入口仍以矢量
轮廓绘制。GPU 位图文字适配由独立的 `skia-gpu-text` crate 负责：使用
`TextAtlasBuilder::new(width, height, max_glyphs)`，随后调用
`insert_layout(&layout, &collection)` 并 `finish()`。先通过
`atlas.layout_quads(&layout, origin)` 生成按 layout 最终位置排列的 glyph quad，再将
`atlas.into_gpu_atlas()` 交给 `GpuCommandEncoder::add_glyph_atlas`，最后显式调用
`draw_glyph_batch(atlas_id, quads, paint)`。adapter 只转换数据，不持有 encoder，也不决定命令
顺序。underline/strike-through 使用独立的
`layout_decoration_batches(&layout, origin)` 转换为连续的 `TextDecorationBatch`；每个 batch 保留
`TextStyleId` 和 target-space `Rect`；Solid/Dashed/Dotted/Wavy 均来自文本层的同一几何生成器，
上层用同一个 style resolver 取得 paint 后逐个录制普通
`fill_rect` 命令。装饰适配位于 `gpu/text/src/decoration.rs`，不依赖 atlas、encoder 或 Metal。
styled layout 使用 `layout_style_batches`，它按视觉顺序返回连续的 `TextGlyphBatch`；每个 batch
保留 `TextStyleId`，上层解析 paint 后逐批调用 `draw_glyph_batch`。

不需要 bitmap atlas 时，可调用 `layout_outline_batches(&layout, &provider, origin)`；它复用
`skia-core::glyph_outline_path`，把 line offset、run origin、justification offset 和 glyph position
固化为 target-space `TextOutlineBatch`。每个 batch 保留 `TextStyleId` 和普通 `Path`，上层逐个
`add_path` 后以 `FillRule::NonZero` 录制 generic `fill_path`。CPU Canvas 与 GPU text adapter 因此
共用同一套 design-unit 到 Q16.16 path 的缩放与溢出规则。

atlas 使用 1 像素 padding、以 `(FontId, GlyphId, font_size_bits, GlyphBitmapFormat)` 去重，并受
尺寸和 glyph 数上限约束；空间不足会明确返回 `ResourceLimit`。`layout_quads` 返回独立、可检查
的 `Vec<GpuGlyphQuad>`；资源 ID、状态、裁剪和绘制顺序仍由 generic GPU encoder 的普通资源协议
管理。通用
`skia-gpu` 命令层只认识 RGBA atlas、quad 和 mask 标记，不依赖 `FontFace`、`GlyphBitmap` 或
`TextLayout`；Metal 等硬件后端也只依赖这层通用协议。

跨帧路径使用有界 `TextAtlasCache`：`get_or_insert_layout` 会复用覆盖所需 glyph 集合的不可变
atlas（允许 superset hit），未命中时按配置重新 raster/pack，达到 `max_atlases` 后淘汰最久未
使用项。返回 `Arc<TextAtlas>`，同一对象可登记到多个 frame encoder；`stats()` 暴露 hit、miss、
eviction 与当前 entry 数。`FontId` 必须继续唯一标识不可变字体实例，字号/format 已包含在 glyph
身份中；目标像素密度变化时上层仍应选择相应字号或清空 cache。

缓存生成的 `GpuGlyphAtlas` 带稳定 `GpuGlyphAtlasKey`。Metal 默认保留最多 8 个、合计 64 MiB
RGBA8 数据对应的 native atlas texture，跨 submit 命中时不再上传；
`set_atlas_cache_capacity` 与 `set_atlas_cache_byte_limit` 可收紧或禁用缓存，
`atlas_cache_stats()` 暴露 hit/upload/eviction。后端会同时核对完整 atlas image，错误复用同一 key
不会替换成不同像素。Metal 已真正批量绘制 Alpha8 mask 与 RGBA8 彩色 glyph；mask glyph
支持局部渐变、颜色滤镜和全部 `BlendMode`，彩色 glyph 保留 atlas RGB，并应用 paint alpha、
颜色滤镜和全部混合模式。
该 API 不做系统字体发现或平台 LCD subpixel filtering。

当前 text 层已负责**单段 shaping、单段落 bidi、按序 fallback、字体 metrics、通用 Unicode
换行、可插拔词典分词/断字、OpenType family/style 元数据和匹配、逻辑/物理对齐、Unicode
可断空格、mixed-script/显式跨脚本 inter-character justification、cluster-safe letter/word spacing、全局
OpenType feature、BCP 47 language-sensitive shaping、grapheme-safe styled paragraph/multiline
layout、line-limit clip/ellipsis、cluster hit testing/caret/selection rectangles、per-span paint ID、
Solid/Dashed/Dotted/Wavy per-span underline/strike-through、GDEF ligature 内部 caret、内置
Knuth-Liang 断字 provider 和轮廓解析**。平台字体发现、generic family 映射与语言字体偏好由
独立 `skia-system-fonts` adapter 提供；variable 实例选择策略仍由调用方负责。
`shape_paragraph` 只接受一个未换行段落；多段内容应使用 `layout_text`。缺少覆盖字体会返回
`MissingGlyph`。当前 Unicode line-break 实现把 SA 复杂上下文字系按普通字母处理；泰文、
老挝文、高棉文和缅甸文需要上层通过 `TextBreakProvider` 接入合适的 `Soft` 词典边界。

需要跑完整 Unicode 一致性语料时，在仓库根目录执行：

```sh
scripts/fetch_unicode_conformance.sh
SKIA_UNICODE_CONFORMANCE_DIR=target/unicode-conformance \
  cargo test -p skia-text --test unicode_conformance -- --ignored
```

下载脚本会校验固定 SHA-256，数据留在 `target/` 而不进入 Git。测试版本跟随当前依赖实际
声明的数据版本：grapheme 为 Unicode 17.0、line break 为 15.0、bidi 为 16.0。升级任一
Unicode 依赖时，必须同时更新对应测试 URL、摘要、版本断言和来源说明。普通
`cargo test -p skia-text` 会执行版本锁定测试，但不会要求本地预先下载约 8 MB 的完整语料。
grapheme 766 条、line break 7,654 条和 bidi 91,707 条现在都是严格全通过门禁；line-break
adapter 补齐测试文件采用的 regex-number tailoring，以及依赖 pair table 未表达的 LB30
东亚宽度例外和 LB30b potential-emoji 规则。任何新增偏差或意外行为变化都会直接失败。

## 先看结论

| 能力 | CPU `Canvas`（即时执行） | `DisplayListBuilder`（录制后由 CPU 回放） | `GpuCommandEncoder`（录制后提交后端） |
| --- | --- | --- | --- |
| 清屏 | `clear` | `clear` | `clear` |
| 保存/恢复状态 | `save` / `restore` | `save` / `restore` | `save` / `restore` |
| 隔离图层 | `save_layer` / `restore` | `save_layer` / `restore` | `save_layer` / `restore`（software reference 与 Metal 均执行） |
| 设置变换 | `set_transform`、`concat` | `set_transform`、`concat_transform` | `set_transform`、`concat_transform` |
| 裁剪 | `clip_rect`、`clip_rect_with_op`、`clip_path` | `clip_rect`、`clip_rect_with_op`、`clip_path` | `clip_rect`、`clip_rect_with_op`、`clip_path` |
| 填充矩形 | `fill_rect` | `fill_rect` | `fill_rect` |
| 填充路径 | `fill_path` | `fill_path` | `fill_path` |
| 描边路径 | `stroke_path`、`stroke_path_with_options` | `stroke_path`、`stroke_path_with_options` | `stroke_path`（显式 `StrokeOptions`） |
| 绘制位图 | `draw_image`、`draw_image_with_sampling` | `draw_image`、`draw_image_with_sampling` | `draw_image`、`draw_image_with_sampling` |
| 绘制文字 | `draw_glyph_run`、`draw_shaped_paragraph`、`draw_text_layout` | `draw_glyph_run`、`draw_positioned_glyph_run`、`draw_shaped_paragraph`、`draw_text_layout` | `layout_outline_batches` + `fill_path` 或 `TextAtlas::layout_quads` + `draw_glyph_batch` |
| 当前实际硬件后端 | CPU 可用 | CPU 可用 | Metal 支持 `clear`、全部 `BlendMode`、渐变/颜色滤镜/box blur 隔离图层、图片、path/stroke、atlas glyph batch 与复杂裁剪；Vulkan 支持完整 portable command 回放、原生 `clear`、staging upload 与真实离屏 readback |

因此，`Canvas` 仍是确定性语义参考实现；`DisplayList` 适合由上层缓存或跨线程传递同一组
CPU 绘制。generic GPU 命令和 Metal 已覆盖表内共享 paint/layer 语义，平台后端不再通过
SourceOver-only 或缺少 layer 模型缩减这些命令。

## 通用约定

- 坐标使用固定点 `Scalar`（Q16.16）；通过 `Scalar::from_i32` 或
  `Scalar::from_ratio` 创建。计算溢出返回 `NumericOverflow`，不会静默截断。
- `Rect::new(left, top, right, bottom)` 必须是正面积矩形（`left < right` 且
  `top < bottom`），坐标系原点在左上。
- `Color` 是 straight-alpha 的 sRGBA8。`BlendMode` 覆盖 Porter-Duff、Plus、Modulate 及
  Multiply/Screen/Overlay 等高级混合；它描述**像素合成**，不是路径的 union/intersect 等
  几何布尔运算。
- `Paint` 可持有最多 8 个有序 stop 的局部坐标线性/径向 `Gradient`，越界坐标通过
  `TileMode::{Clamp, Repeat, Mirror}` 处理；paint color 的 alpha 调制渐变，之后再应用可选
  `ColorFilter`。`ColorMatrix` 使用确定性的 Q16.16 4×5 straight-RGBA 变换。
- 变换是仿射矩阵 `(a, b, c, d, e, f)`。`set_transform` 替换当前变换；Canvas 的
  `concat(next)`、DisplayList/GPU encoder 的 `concat_transform(next)` 均表示先应用当前
  变换、再应用 `next`。
- `save` 保存变换和裁剪，`restore` 恢复最近一层；没有匹配的 `save` 会返回
  `RestoreUnderflow`。Canvas 默认最多 256 层，可由 `SurfaceLimits` 收紧。
- `save_layer(options)` 同时保存状态并切换到透明隔离目标；`restore` 可在受限 bounds 内以
  opacity/blend mode 合成，也可先应用逐像素颜色滤镜或透明边界的可分离 box blur。CPU 图层
  像素由 `SurfaceLimits::max_bytes` 和同一 save-depth 上限共同约束。
- `clear` 总是作用于整个目标，忽略当前变换和裁剪。

## 1. CPU Canvas：下层即时执行路径

调用顺序为：`Surface::new(...)` → `surface.canvas()` → 以下命令。`Canvas` 持有对
`Surface` 的可变借用，结束后可经 `Surface::pixels()` 读取紧密排列的 RGBA8 像素。

### 状态命令

| 命令 | 作用 | 现有边界 |
| --- | --- | --- |
| `clear(color)` | 用一个颜色覆盖整个目标。 | 忽略状态；无返回值。 |
| `save()` | 压入当前变换与裁剪。 | 受 `max_save_depth` 限制。 |
| `save_layer(options)` | 压入状态并开始透明隔离图层。 | restore 时应用 bounds、opacity、blend mode 和可选 color/box-blur filter；分配计入 surface 内存预算。 |
| `restore()` | 弹出并恢复最近状态。 | 空栈报错。 |
| `set_transform(transform)` | 替换后续绘制使用的变换。 | 不会与旧变换相乘。 |
| `concat(transform)` | 把变换追加到当前变换。 | 可能因固定点计算溢出失败。 |
| `clip_rect(ClipRect::new(rect))` | 用变换后的矩形和当前裁剪求交。 | 轴对齐交集保留 scissor 快路径；旋转或错切时使用确定性的像素 mask。 |
| `clip_rect_with_op(clip, op)` | 以 `Intersect` 或 `Difference` 应用矩形裁剪。 | 非轴对齐或差集使用 mask；当前为中心点采样的硬边裁剪。 |
| `clip_path(path, rule, op)` | 按 `EvenOdd` 或 `NonZero` 和当前裁剪求交/差。 | 曲线复用 `skia-tessellation` 的固定步数展平；mask 为每像素一字节。 |

### 图形命令

| 命令 | 作用 | 已确定的行为/边界 |
| --- | --- | --- |
| `fill_rect(rect, paint)` | 填充变换后的矩形。 | 通过通用路径光栅化，允许旋转/错切；渐变在逆变换后的局部坐标中求值。 |
| `fill_path(path, rule, paint)` | 按 `EvenOdd` 或 `NonZero` 填充路径。 | 二、三次贝塞尔由 `skia-tessellation` 以固定步数展平；开放轮廓在填充时会隐式闭合。 |
| `stroke_path(path, width, paint)` | 兼容描边入口。 | 宽度必须为正；保持 Center alignment、round cap + round join 行为。 |
| `stroke_path_with_options(path, options, paint)` | 完整描边路径。 | 复用 `skia-tessellation` 的固定步数曲线展平与共享三角网格；支持 Center/Inside/Outside、butt/round/square cap、miter/round/bevel join、miter limit，以及偶数正值 on/off dash pattern 和规范化 phase；Inside/Outside 要求闭合、非退化轮廓。 |
| `stroke_to_path(path, options, transform)` | 将描边扩展为普通 `Path`。 | facade 函数；以指定 affine transform 展平并生成可用 `FillRule::NonZero` 填充的确定性三角形路径，保留 alignment/cap/join/dash，适合复用到裁剪、缓存或后续 path 流程。 |
| `path_boolean(subject, clip, op, input_rule, transform, limits)` | 对两条路径做 union、intersection、difference 或 XOR。 | facade 函数；在 Q16.16 整数坐标上处理洞和自交，空集合返回 `None`，非空结果使用 `FillRule::NonZero`；调用方必须按预算提供或收紧 `PathBooleanLimits`。 |
| `trim_path(path, effect, transform, limits)` | 按归一化弧长裁剪每个轮廓。 | `TrimPathEffect` 的 start/end 位于 `[0,1]`；start 大于 end 时跨轮廓 seam，部分结果保持开放，完整 `[0,1]` 保留显式闭合；空结果返回 `None`。 |
| `corner_path(path, effect, transform, limits)` | 将折线拐角替换为确定性的二次曲线。 | `CornerPathEffect` 要求正半径；每侧裁剪距离不超过半径和相邻边长的一半，开放端点不变，显式闭合轮廓继续闭合；空结果返回 `None`。 |
| `discrete_path(path, effect, transform, limits)` | 均匀重采样轮廓并沿局部法线做种子扰动。 | `DiscretePathEffect` 要求正的最大分段长度和非负偏移量；纯整数哈希保证同输入/seed 的 Q16.16 结果稳定，开放端点固定，闭合 seam 只采样一次并保留 `Close`。 |
| `dash_path(path, effect, transform, limits)` | 将路径中心线分成可见 dash 片段。 | `DashPathEffect` 接受偶数个正 on/off 长度并规范化 phase；曲线复用固定步数展平，闭合 seam 上连续的可见区间保持连接。 |
| `apply_path_effect(path, effect, transform, limits)` | 通过统一 `PathEffect` 接口执行一个路径效果。 | trim/corner/discrete/dash 以及对象化 compose/sum 已实现该可扩展接口；效果移除全部几何时返回 `None`，每次执行都受 `PathEffectLimits` 约束。 |
| `compose_path_effects(path, effects, transform, limits)` | 从左到右组合多个路径效果。 | 输入 transform 只在第一阶段应用，后续阶段使用 identity，避免重复变换；每阶段独立执行相同资源预算；空效果列表是受输出 verb 上限约束的 transform-only 管线。 |
| `ComposePathEffect::new(outer, inner)` / `SumPathEffect::new(first, second)` | 将组合本身作为一个可嵌套 `PathEffect`。 | compose 先执行 inner 再执行 outer；sum 对同一变换输入独立求值并拼接轮廓，最终输出仍受统一 verb 上限约束。 |
| `draw_image(image, destination, opacity, blend_mode)` | 以兼容默认值绘制 RGBA8 图片。 | 等价于 `SamplingOptions::NEAREST`；`opacity` 只乘源 alpha；旋转、反射与错切使用 checked inverse mapping。 |
| `draw_image_with_sampling(image, destination, opacity, blend_mode, sampling)` | 以显式采样配置绘制 RGBA8 图片。 | 支持任意可逆 affine transform 下 texel-center、clamp-to-edge 的 Nearest/Linear；CPU 使用确定性整数双线性插值。 |
| `draw_image_with_paint(image, destination, opacity, paint, sampling)` | 用完整图片 paint 绘制 RGBA8 图片。 | paint alpha 调制图片 opacity，颜色滤镜作用于采样颜色，blend mode 控制合成；paint RGB/gradient 不会误作图片 tint。 |
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
| `SaveLayer` / `save_layer` | `SaveLayerOptions` | 保存状态并录制隔离图层起点；CPU 回放在匹配 `Restore` 时过滤并合成。 |
| `Restore` / `restore` | 无 | 恢复状态。 |
| `ClipRect` / `clip_rect`、`clip_rect_with_op` | `Rect`、`ClipOp` | 兼容入口为交集；显式入口支持交集/差集。 |
| `ClipPath` / `clip_path` | `PathId`、`FillRule`、`ClipOp` | 录制任意路径交集/差集，CPU 回放解析列表自有路径。 |
| `SetTransform` / `set_transform` | `Transform` | 替换变换。 |
| `ConcatTransform` / `concat_transform` | `Transform` | 叠加变换。 |
| `FillRect` / `fill_rect` | `Rect`、`Paint` | 填充逻辑矩形；回放时沿用当时的变换和裁剪状态。 |
| `FillPath` / `fill_path` | `PathId`、`FillRule`、`Paint` | 填充已登记的路径。 |
| `StrokePath` / `stroke_path`、`stroke_path_with_options` | `PathId`、`StrokeOptions`、`Paint` | 兼容入口生成 round/round options；显式入口保留完整 cap/join/miter/dash 几何。 |
| `DrawImage` / `draw_image`、`draw_image_with_sampling` | `ImageId`、目标 `Rect`、`u8` opacity、`Paint`、`SamplingOptions` | 兼容入口记录 Nearest；显式入口保留 Nearest/Linear，回放使用 paint alpha、color filter 与 blend mode。 |
| `DrawGlyphRun` / `draw_glyph_run` | `GlyphRunId`、`Paint` | 绘制已登记的整形字形序列。 |
| `DrawPositionedGlyphRun` / `draw_positioned_glyph_run` | `GlyphRunId`、baseline origin、每 glyph Q16.16 X offset、`Paint` | 保留 line/run origin 与 justification 位移；paragraph/layout 便利入口以它为执行原语。 |

`draw_shaped_paragraph[_with_styles]` 与 `draw_text_layout[_with_styles]` 是 builder 便利入口：前者
登记并定位每个 visual run，后者再把 Solid/Dashed/Dotted/Wavy 装饰展开为普通 `FillRect`。
高层展开是事务性的，失败不会遗留部分命令或孤立 glyph resource。

资源须先经 `add_path`、`add_image` 或 `add_glyph_run` 登记，取得仅在该列表中有效的 ID；
`finish()` 后发布列表。构建器的 `max_items` 同时限制**命令数及每一种资源数**。

### 与 Canvas 的差异

- 命令本身不携带“当时状态”的快照；回放时按命令顺序维护状态。因此保存/恢复顺序是
  列表语义的一部分。
- 回放使用 Canvas，所以最终约束与 CPU Canvas 一致，包括完整裁剪状态、图片 affine inverse mapping、
  描边样式和文字轮廓解析要求。

## 3. GPU：下层编码与提交路径

`GpuCommandEncoder` 是另一套后端中立的命令表，不是 `DisplayList` 的执行器。流程为：
创建 encoder → 登记资源 → 录制命令 → `finish()` → 由实现 `GpuBackend` 的后端
`submit` 到 `GpuSurfaceDescriptor` 指定大小的表面。

| `GpuCommand` / Encoder 方法 | 参数 | 说明 |
| --- | --- | --- |
| `Clear` / `clear` | `Color` | 清空完整目标，不受裁剪影响。 |
| `SaveLayer` / `save_layer`、`RestoreLayer` / `restore` | `SaveLayerOptions` | 录制隔离图层边界；software reference 与 CPU 语义一致，硬件后端必须执行或 fail-closed。 |
| 状态 / `save`、`restore`、`set_transform`、`concat_transform` | 同名状态参数 | 这些方法修改 encoder 状态；每个绘制命令会记录当时的 transform、scissor 和复杂裁剪 ID。 |
| 裁剪 / `clip_rect`、`clip_rect_with_op`、`clip_path` | `Rect`/`GpuPathId`、`FillRule`、`ClipOp` | 轴对齐矩形交集走 target-space scissor；其余裁剪形成不可变、可共享的 `GpuClipId` 父链。 |
| `FillRect` / `fill_rect` | `Rect`、`Paint` | 填充矩形。 |
| `FillPath` / `fill_path` | `GpuPathId`、`FillRule`、`Paint` | 填充已登记路径。 |
| `StrokePath` / `stroke_path` | `GpuPathId`、`StrokeOptions`、`Paint` | 描边已登记路径，并快照 alignment/cap/join/dash、transform、scissor 与复杂裁剪 ID。 |
| `DrawImage` / `draw_image`、`draw_image_with_sampling`、`draw_image_with_paint` | `GpuImageId`、目标 `Rect`、`u8` opacity、`Paint`、`SamplingOptions` | 兼容入口从 blend mode 构造白色 paint；完整入口快照 paint alpha、color filter、blend mode 与 Nearest/Linear 采样。 |
| `DrawGlyphs` / `draw_glyph_batch` | `GpuGlyphAtlasId`、`Vec<GpuGlyphQuad>`、`Paint` | 一次绘制一个已登记 atlas 的定位 glyph quad。 |

GPU encoder 要求先调用 `add_path` / `add_image` / `add_glyph_atlas` 登记对应资源。
`GpuCommandLimits` 可分别限制命令、路径、图片、复杂裁剪节点、状态栈深度和单批 glyph 数。
复杂裁剪节点只引用 encoder 自有 Path 资源和父节点，不复制几何或 mask；`save/restore` 只复制
轻量 ID。轴对齐矩形交集为空时，后续绘制命令不会被录制。

`TextLayout` 到 `DrawGlyphs` / decoration `FillRect` 的转换不属于 GPU encoder：它只存在于
`skia-gpu-text`，glyph 通过 `TextAtlasBuilder` → `TextAtlas::layout_quads` → generic atlas
registration/batch draw 完成，装饰通过 `layout_decoration_batches` → generic `fill_rect` 完成。
因此 generic GPU backend 可独立演进，文字 raster/cache 策略也不会进入 Metal 等平台 crate。
Cargo 层也保持相同边界：`skia-core` 默认开启 `text` feature 以保留 DisplayList glyph-run
能力，而 `skia-gpu` 与 `skia-metal` 使用 `default-features = false`，所以单独构建 Metal 不会
拉入 `skia-text`、`rustybuzz`、`swash` 或 Unicode shaping 依赖；只有 `skia-gpu-text` adapter
依赖完整 text 能力。

### GPU 当前缺口

- GPU 命令层没有专用 `draw_glyph_run` 或文字装饰命令；`skia-gpu-text` 已可把矢量 glyph 降为
  generic `FillPath`、把 bitmap glyph 降为 atlas batch，并把四种装饰线型降为普通 `FillRect`。
  generic paint/command 已保留渐变、颜色滤镜、图片 paint 和隔离图层；复杂裁剪也已进入
  generic GPU 和 Metal 支持的 draw 路径。`SaveLayer` 同时快照 transform/scissor/complex clip，
  因此带逻辑 bounds 的 restore 不依赖相邻绘制命令的偶然状态。
- `SoftwareGpuBackend` 能用 CPU Canvas 回放上述 GPU 命令，主要用于一致性测试，并不是真正的
  硬件 GPU 实现。
- `MetalBackend` 会真实执行 `Clear`，把 `FillRect` 展开为两个 triangle，并上传 glyph atlas 后
  批量绘制 mask/color glyph；`FillPath` 使用同一条有界、固定 16 段曲线展平路径生成临时 R8
  fill mask，因此支持凹轮廓、洞和 `EvenOdd` / `NonZero` 规则。复杂裁剪按被引用的 `GpuClipId`
  父链生成临时 R8 texture：每个节点在一次 submit 中只生成一次，子节点采样父 mask；路径 fill
  mask 会再与最终复杂裁剪 mask 相交。硬件像素读回测试覆盖 transform、scissor、alpha blend 与
  两种 glyph。`DrawImage` 以 Nearest 或 Linear 采样 RGBA8 图片，支持任意仿射变换、opacity、
  color filter、scissor 与复杂裁剪。`StrokePath` 复用共享的描边三角形并由 Metal 光栅化为 R8
  覆盖 mask，再进行最终 blend 与复杂裁剪。所有 draw 在提交内先以 blit 快照当前目标，fragment
  shader 再统一执行 straight/premultiplied 转换及全部 29 个 `BlendMode`，所以高级混合不会被
  错映射成固定函数 SourceOver。线性/径向渐变按插值的 local position 求值；颜色矩阵或 blend
  filter 在合成前执行。`SaveLayer` 使用真实 RGBA8 Metal texture 栈，颜色滤镜和两遍可分离
  box blur 在 restore 前执行，随后按保存的 bounds/opacity/blend/clip 合回父目标。
- `VulkanBackend` 动态加载系统 Vulkan loader，选择 graphics queue，创建 optimal-tiled RGBA8
  image，以原生 transfer clear 和完整 portable command 回放覆盖所有命令，并用 host-visible
  staging buffer 完成上传和精确像素读回。目标内容可跨提交保留；尚未创建 swapchain 或完全
  原生的 shader/descriptor 绘制管线。

## 4. 路径构造能力（为下层绘制准备资源）

路径不是 `Canvas` 状态命令，但它决定 `fill_path` 和 `stroke_path` 可以表达哪些图形。使用
`PathBuilder::new(max_verbs)` 创建，并以 `finish()` 发布不可变 `Path`。

| 分组 | 方法 | 说明 |
| --- | --- | --- |
| 基本轮廓 | `move_to`、`line_to`、`quad_to`、`conic_to`、`cubic_to`、`close` | 直线、二次/有理二次/三次贝塞尔；除 `move_to` 外必须有活跃轮廓。 |
| 基本形状 | `add_rect`、`add_oval`、`add_circle`、`add_round_rect` | oval/圆角使用确定性的三次贝塞尔近似；圆半径必须正，圆角半径不得为负且会夹到矩形半宽/半高。 |
| 描边几何 | `StrokeOptions`、`StrokeAlign`、`StrokeCap`、`StrokeJoin` | 定义 Center/Inside/Outside alignment、butt/round/square cap、miter/round/bevel join、miter limit 和规范化的偶数正值 dash pattern/phase；非居中 alignment 仅接受闭合、非退化轮廓。 |
| 多边形 | `add_polygon` | 接受开放或闭合多边形；开放至少两个点，闭合至少三个点。 |
| 椭圆弧 | `add_arc` | 从 `ArcStart` 开始、按 `ArcDirection` 画 1–4 个 90° 段。 |
| 任意角度弧 | `add_arc_degrees`、`arc_to` | `Angle` 使用顺时针 canvas 度数；扫角不能为 0，绝对值不能超过一整圈；最多拆成四段三次贝塞尔。`arc_to` 会在需要时先连一条直线到弧起点。 |
| 旋转椭圆弧 | `add_rotated_arc_degrees`、`arc_to_rotated` | 在椭圆中心旋转后输出三次贝塞尔段；参数仍使用确定性 Q16.16 角度。 |
| 组合/查询 | `append_path`、`Path::reversed`、`Path::transformed`、`Path::bounds`、`Path::tight_bounds` | 支持追加、反向、生成变换副本、控制点保守边界和多项式贝塞尔 extrema-aware 保守边界。 |

## 用这份表排查不足

优先确认目标调用路径属于哪一层；不要把“CPU 已可画”误判为“DisplayList 或 Metal 已可画”。
本轮上层缺口的收口状态如下：

1. gradient、color filter、save-layer 和全部 blend mode 已由 CPU/software reference 与 Metal
   执行；Metal 的硬件像素门禁在有真实 device 时运行，构建环境至少必须通过 `.metal` 编译；
2. Vulkan 已实现完整 portable command 回放、原生 offscreen clear、跨提交内容保留、staging
   upload/readback 与同步；swapchain 和完全原生的 shader/descriptor 绘制管线仍是独立后续工作；
3. 系统字体发现、generic-family 映射和语言字体偏好已由独立 `skia-system-fonts` adapter
   提供；纯 Rust `skia-text` 仍不接触平台目录；
4. 内置 Knuth-Liang 断字词典已由 `BuiltinTextBreakProvider` 提供，并保留自定义
   `TextBreakProvider` 注入边界；
5. 当前纯 Rust WebP encoder 明确只支持 VP8L；有损 WebP 由独立实现负责，本轮不改；
6. `path_boolean`、`stroke_to_path`、trim/corner/discrete/dash 与对象化 compose/sum
   `PathEffect` 已提供；需要沿路径重复任意图形的 1D/2D stamping 属于独立场景图 API，
   不再列为现有绘制契约缺口。

源码入口：Geometry 在 `geometry/src/lib.rs`，Path 在 `path/src/lib.rs`，CPU Canvas 入口在
`cpu/src/canvas.rs`，复杂裁剪 mask 在 `cpu/src/clip.rs`；共享的描边归一化、虚线分段和
cap/join/miter 命中算法在 `tessellation/src/stroke.rs`，CPU 的设备像素边界计算在
`cpu/src/stroke.rs`，路径布尔运算适配在 `tessellation/src/boolean.rs`，path effects 在
`tessellation/src/path_effect.rs`。文字仍由单一 `skia-text` crate 负责：基础 glyph/run 类型在
`text/src/types.rs`，outline 契约在 `text/src/outline.rs`，错误定义在 `text/src/error.rs`，
字体加载与 shaping 在 `text/src/font.rs`，collection/fallback 在 `text/src/collection.rs`，
布局与编辑几何在 `text/src/layout.rs`，共享装饰条带在 `text/src/decoration.rs`；`text/src/lib.rs`
只组织模块并维持公开 re-export。
DisplayList 在 `core/src/display_list.rs`，GPU 命令层在 `gpu/src/lib.rs`，GPU 文字适配 package 位于
`gpu/text/`（atlas packing、layout quad、装饰矩形和矢量 outline 转换分别在 `src/atlas.rs`、
`src/layout.rs`、`src/decoration.rs`、`src/outline.rs`），Metal
后端无关的固定步数曲线展平在 `tessellation/src/flatten.rs`；CPU 将其输出接到
`cpu/src/canvas.rs` 的栅格轮廓。Metal 后端提交在 `gpu/metal/src/lib.rs`，Metal mask 生命周期在
`gpu/metal/src/clip.rs`，`gpu/metal/src/clip_geometry.rs` 只把共享折线轮廓转换为裁剪边，mask 与
draw shader 在 `shaders/solid_rect.metal`。
Vulkan loader/device/queue 生命周期在 `gpu/vulkan/src/context.rs`，offscreen image、layout barrier
与 staging readback 在 `gpu/vulkan/src/surface.rs`；Windows 强制设备测试见 `gpu/vulkan/README.md`。

## Rust 工具链维护

本仓库使用 `rustup` 管理 Cargo 与 Rust 工具链。`skia-rs/rust-toolchain.toml`
会让 `cargo` / `rustc` 自动选择 `stable`，并安装 `clippy` 和 `rustfmt`；
`skia-rs/Cargo.toml` 的 `rust-version = "1.89"` 是本仓库当前的
最低支持版本（由纯 Rust `mozjpeg-rs` 要求）。更新 stable 工具链（以及 Cargo）后，用以下
命令确认版本：

```sh
cd skia-rs
rustup update stable
cargo --version
rustc --version
```

工作区验证会构建全部内部 crate 与可选后端：

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

完整的测试分层、上游 Skia 资产的复用/许可边界、golden 更新协议和 CI 分工见
[`TESTING_STRATEGY.md`](TESTING_STRATEGY.md)。GitHub Actions 的 portable job 会运行以上
format、Clippy、workspace tests 和 SHA-256 锁定的 Unicode conformance 下载；macOS job 运行
Metal 的 device-optional 测试。需要真实设备像素门禁时，必须配置受保护的 macOS runner，并显式
设置 `SKIA_REQUIRE_METAL_DEVICE=1`；不得把无 device 的跳过当成硬件通过。

CPU 与 software-GPU 共享的自有像素场景位于 `gpu/tests/support/render_cases.rs`；普通测试将它们逐
RGBA8 像素与 `tests/golden/` 中审查过的 fixture 比较。只有在明确审查像素/PNG diff 后，才可更新
golden：

```sh
SKIA_UPDATE_GOLDENS=1 scripts/regenerate_goldens.sh
cargo test -p skia-gpu --features software --test render_oracle
```

只验证上层公开图片 codec 契约时，运行根 crate 的 facade 集成测试；该测试不直接引用
`skia-codec`、`image` 或具体格式实现 crate：

```sh
cargo test --test codec_api
```

只验证字体加载、UTF-8 shaping、轮廓解析和 CPU 文字绘制链路时，运行：

```sh
cargo test -p skia-text -p skia-core -p skia-cpu --tests
cargo test -p skia-text decoration::tests::patterns_expand_with_deterministic_phase -- --exact
cargo test -p skia-cpu --test font text_decoration_patterns_share_resolved_cpu_geometry -- --exact
cargo test -p skia-cpu --test font display_list_expands_layout_runs_and_decorations_transactionally -- --exact
cargo test -p skia-gpu-text --test adapter text_adapter_expands_all_decoration_patterns -- --exact
```

只验证 GPU glyph atlas 的资源边界、software reference replay，以及 macOS Metal shader 的
真实像素读回时，运行：

```sh
cargo test -p skia-gpu --all-features
cargo test -p skia-gpu-text
cargo test -p skia-metal
cargo test -p skia-vulkan
cargo check -p skia-core --no-default-features
cargo tree -p skia-metal --edges normal
```

要求 GPU text 的 Metal 端到端用例必须取得真实 device、不能静默跳过时，运行：

```sh
SKIA_REQUIRE_METAL_DEVICE=1 cargo test -p skia-gpu-text text_adapter_draws_styled_glyphs_and_decorations_on_metal -- --exact
SKIA_REQUIRE_METAL_DEVICE=1 cargo test -p skia-metal
```

Windows PowerShell 上强制真实 Vulkan loader/device 与 validation layer 的命令为：

```powershell
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
$env:SKIA_VULKAN_VALIDATION = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

Vulkan 后端的范围和 Windows 验证说明见 `gpu/vulkan/README.md`。

`skia-metal` 测试需要 macOS Metal device 和 Xcode command-line shader tools；没有默认 Metal
device 时普通硬件用例会跳过；设置 `SKIA_REQUIRE_METAL_DEVICE=1` 后 device 不可用会令上述
GPU text 端到端测试和 `skia-metal` 硬件测试直接失败。shader library 创建或编译异常同样直接失败。最后一条 dependency tree
中不应出现 `skia-text`、`rustybuzz` 或 `swash`；出现任一项都表示 GPU/text 边界发生回归。
`skia-vulkan` 普通测试同样只在 loader/device 不可用时跳过；`SKIA_REQUIRE_VULKAN_DEVICE=1`
将其改为硬失败，`SKIA_VULKAN_VALIDATION=1` 还会要求系统提供
`VK_LAYER_KHRONOS_validation`。
