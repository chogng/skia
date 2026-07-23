use skia_core::{ClipOp, FillRule, Rect, Transform};
use skia_image::Image;

use crate::{GpuCommandError, GpuCommandErrorCode};

/// Opaque, command-buffer-local identifier for one immutable vector path.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuPathId(pub(crate) u32);

/// Opaque, command-buffer-local identifier for one immutable clip node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuClipId(pub(crate) u32);

/// Opaque, command-buffer-local identifier for one immutable RGBA8 image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuImageId(pub(crate) u32);

/// Opaque, command-buffer-local identifier for one immutable glyph atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphAtlasId(pub(crate) u32);

/// Stable caller-owned identity for reusing one immutable atlas across submissions.
///
/// Backends verify the atlas content before reusing a resource, so accidental
/// key reuse cannot substitute different pixels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphAtlasKey(u64);

impl GpuGlyphAtlasKey {
    /// Creates one stable atlas cache identity.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the caller-owned identity value.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Integer pixel rectangle inside one glyph atlas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuAtlasRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl GpuAtlasRect {
    /// Creates one non-empty atlas pixel rectangle.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, GpuCommandError> {
        if width == 0 || height == 0 {
            return Err(GpuCommandError::new(GpuCommandErrorCode::InvalidResource));
        }
        x.checked_add(width)
            .and_then(|_| y.checked_add(height))
            .ok_or(GpuCommandError::new(GpuCommandErrorCode::NumericOverflow))?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns the left atlas pixel.
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Returns the top atlas pixel.
    pub const fn y(self) -> u32 {
        self.y
    }

    /// Returns the atlas region width.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the atlas region height.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Immutable RGBA8 atlas consumed by generic glyph batch commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuGlyphAtlas {
    image: Image,
    cache_key: Option<GpuGlyphAtlasKey>,
}

impl GpuGlyphAtlas {
    /// Wraps one prepacked RGBA8 image for low-level glyph batch recording.
    pub const fn from_image(image: Image) -> Self {
        Self {
            image,
            cache_key: None,
        }
    }

    /// Associates this immutable atlas with a cross-submission cache identity.
    pub const fn with_cache_key(mut self, cache_key: GpuGlyphAtlasKey) -> Self {
        self.cache_key = Some(cache_key);
        self
    }

    /// Borrows the upload-ready straight-alpha RGBA8 atlas image.
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Returns the optional cross-submission cache identity.
    pub const fn cache_key(&self) -> Option<GpuGlyphAtlasKey> {
        self.cache_key
    }
}

/// One positioned atlas quad inside a batched glyph command.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuGlyphQuad {
    source: GpuAtlasRect,
    destination: Rect,
    mask: bool,
}

impl GpuGlyphQuad {
    /// Creates one atlas sample and logical destination pair.
    pub const fn new(source: GpuAtlasRect, destination: Rect, mask: bool) -> Self {
        Self {
            source,
            destination,
            mask,
        }
    }

    /// Returns the source rectangle inside the atlas.
    pub const fn source(self) -> GpuAtlasRect {
        self.source
    }

    /// Returns the logical destination rectangle.
    pub const fn destination(self) -> Rect {
        self.destination
    }

    /// Returns whether atlas alpha should tint the command paint color.
    pub const fn is_mask(self) -> bool {
        self.mask
    }
}

/// Geometry retained by one immutable complex clip node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuClipGeometry {
    /// One logical rectangle.
    Rect(Rect),
    /// One command-buffer-local path and its containment rule.
    Path {
        /// Path resource local to the command buffer.
        path: GpuPathId,
        /// Containment rule used by the clip.
        rule: FillRule,
    },
}

/// One immutable node in a command buffer's persistent complex-clip chain.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuClipNode {
    pub(crate) parent: Option<GpuClipId>,
    pub(crate) geometry: GpuClipGeometry,
    pub(crate) op: ClipOp,
    pub(crate) transform: Transform,
}

impl GpuClipNode {
    /// Returns the preceding complex clip node, if any.
    pub const fn parent(self) -> Option<GpuClipId> {
        self.parent
    }

    /// Returns the logical geometry retained by this node.
    pub const fn geometry(self) -> GpuClipGeometry {
        self.geometry
    }

    /// Returns the boolean operation applied by this node.
    pub const fn op(self) -> ClipOp {
        self.op
    }

    /// Returns the logical-to-target transform captured for this geometry.
    pub const fn transform(self) -> Transform {
        self.transform
    }
}
