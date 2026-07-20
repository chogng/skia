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

当前 `TextStyleSpan` 只携带字体实例和字号，不携带颜色或装饰：颜色属于 `Paint`，上层需要按
`ShapedRun` 拆分绘制时选择不同 paint。跨行 styled text 直接调用
`layout_styled_text(text, spans, options)`；spans 的覆盖、顺序、grapheme 边界和 FontId
约束与 styled paragraph 相同。每个候选行都会重新执行 bidi/fallback/shaping，不会直接切开
已有 glyph run。

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
非空行。styled layout 的行 metrics 取该行实际 runs 的最大值；连续 hard break 产生的空行
使用其逻辑行首 span 的 preferred face 和字号，尾随换行空行使用最后一个 span。

需要语言词典分词或断字时，由上层实现
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
`max_shaping_attempts` 共享工作上限；核心不捆绑具体语言词典，上层负责词典版本、缓存和
语言回退。

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
`with_justify_last_line(true)`。若行内没有可扩展空格，`Justify` 会自动改在 Han、Kana、
Hangul 与 Bopomofo 的相邻 shaping cluster 之间分配剩余宽度；组合 mark、ligature 内部和
CJK 标点不会被拆开，跨 fallback/styled run 的相邻 CJK cluster 仍可扩展。若这两类 slot
都不存在，才回退为逻辑 `Start`。
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

请求的指标表缺失时，layout 返回 `MissingDecorationMetrics`，不会猜测平台相关默认值；字体
给出非正 thickness 时返回 `InvalidFontData`。空行不产生装饰，也不要求字体提供指标。
`Canvas::draw_text_layout` 在 glyph 之后用同一个 `Paint` 绘制实线装饰。当前装饰粒度是整个
layout，尚无 per-span 颜色/粗细、波浪线/虚线，也没有专用 DisplayList layout 命令；上层若
展开录制，需用字体指标和 line 的最终横向范围自行录制对应路径或矩形。

`FontFace` 内部使用纯 Rust `rustybuzz` 完成 shaping，并通过其 `ttf-parser` 解析矢量轮廓；
字体字节由 face 自身不可变持有。轮廓的字体坐标会转换为 canvas 向下为正的坐标，再复用普通
path fill 管线。空格等没有矢量轮廓的字形可以参与 shaping 和 advance，但绘制时不产生路径。

当前 text 层已负责**单段 shaping、单段落 bidi、按序 fallback、字体 metrics、通用 Unicode
换行、可插拔词典分词/断字、OpenType family/style 元数据和匹配、逻辑/物理对齐、Unicode
可断空格与 CJK inter-character justification、cluster-safe letter/word spacing、全局
OpenType feature、BCP 47 language-sensitive shaping、grapheme-safe styled paragraph/multiline
layout、line-limit clip/ellipsis、cluster hit testing/caret/selection rectangles、实线
underline/strike-through、GDEF ligature 内部 caret 和轮廓解析**，但不负责平台字体发现、generic family 映射、
variable 实例选择策略、语言偏好、内置词典/断字算法、通用跨脚本 inter-character
justification、per-span paint/装饰或装饰线变体。
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
   通用换行、可插拔词典断点、hyphenation、对齐、Unicode 空格/CJK inter-character
   justification 与实线装饰，
   并支持 variable/feature 实例、language-sensitive shaping、cluster-safe letter/word
   spacing、styled paragraph/layout、line-limit clip/ellipsis 与
   cluster/GDEF-ligature hit testing、caret 与 selection rectangles；但仍没有系统字体发现、内置语言词典、
   variable 实例选择策略、通用跨脚本 inter-character justification、
   per-span paint/装饰与完整排版；
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
