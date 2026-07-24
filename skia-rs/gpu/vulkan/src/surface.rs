use std::sync::Arc;

use ash::vk;
use skia_core::{Color, ImageShader, SamplingFilter, TileMode};
use skia_gpu::GpuSurfaceDescriptor;
use skia_image::Image;

use crate::{VulkanError, VulkanErrorCode, context::VulkanContext};

/// Vulkan-owned offscreen RGBA8 storage target.
pub struct VulkanSurface {
    context: Arc<VulkanContext>,
    pixels: PixelBuffer,
    descriptor: GpuSurfaceDescriptor,
    initialized: bool,
}

impl VulkanSurface {
    pub(crate) fn new(
        context: Arc<VulkanContext>,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self, VulkanError> {
        let pixels = PixelBuffer::new(context.clone(), descriptor)?;
        Ok(Self {
            context,
            pixels,
            descriptor,
            initialized: false,
        })
    }

    /// Returns the portable descriptor used to allocate this target.
    pub const fn descriptor(&self) -> GpuSurfaceDescriptor {
        self.descriptor
    }

    /// Reads the completed target as tightly packed row-major straight RGBA8.
    pub fn read_rgba8(&self) -> Result<Vec<u8>, VulkanError> {
        if !self.initialized {
            return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
        }
        self.pixels.read_rgba8()
    }

    pub(crate) fn belongs_to(&self, context: &Arc<VulkanContext>) -> bool {
        Arc::ptr_eq(&self.context, context)
    }

    pub(crate) const fn pixels(&self) -> &PixelBuffer {
        &self.pixels
    }

    pub(crate) fn mark_initialized(&mut self) {
        self.initialized = true;
    }
}

pub(crate) struct PixelBuffer {
    context: Arc<VulkanContext>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    byte_len: vk::DeviceSize,
    descriptor: GpuSurfaceDescriptor,
}

impl PixelBuffer {
    pub(crate) fn new(
        context: Arc<VulkanContext>,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self, VulkanError> {
        let byte_len = pixel_byte_length(descriptor, VulkanErrorCode::SurfaceAllocationFailed)?;
        let (buffer, memory, _) = allocate_buffer(
            &context,
            byte_len,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::empty(),
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            VulkanErrorCode::SurfaceAllocationFailed,
        )?;
        Ok(Self {
            context,
            buffer,
            memory,
            byte_len,
            descriptor,
        })
    }

    pub(crate) const fn handle(&self) -> vk::Buffer {
        self.buffer
    }

    pub(crate) const fn byte_len(&self) -> vk::DeviceSize {
        self.byte_len
    }

    pub(crate) const fn descriptor(&self) -> GpuSurfaceDescriptor {
        self.descriptor
    }

    pub(crate) fn clear(&self, color: Color) -> Result<(), VulkanError> {
        let [red, green, blue, alpha] = color.channels();
        let packed = u32::from_le_bytes([red, green, blue, alpha]);
        self.context.submit_commands(
            |command_buffer| {
                let barriers = [vk::BufferMemoryBarrier::default()
                    .src_access_mask(
                        vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE,
                    )
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .buffer(self.buffer)
                    .offset(0)
                    .size(self.byte_len)];
                // SAFETY: this buffer has TRANSFER_DST usage, the full range
                // is four-byte aligned, and the barrier orders prior writes.
                unsafe {
                    self.context.device().cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &barriers,
                        &[],
                    );
                    self.context.device().cmd_fill_buffer(
                        command_buffer,
                        self.buffer,
                        0,
                        self.byte_len,
                        packed,
                    )
                };
                Ok(())
            },
            VulkanErrorCode::SubmissionFailed,
        )
    }

    fn read_rgba8(&self) -> Result<Vec<u8>, VulkanError> {
        let (staging, memory, coherent) = allocate_buffer(
            &self.context,
            self.byte_len,
            vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT | vk::MemoryPropertyFlags::HOST_CACHED,
            VulkanErrorCode::ReadbackFailed,
        )?;
        let result = self
            .context
            .submit_commands(
                |command_buffer| {
                    let source_barrier = [vk::BufferMemoryBarrier::default()
                        .src_access_mask(
                            vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE,
                        )
                        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .buffer(self.buffer)
                        .offset(0)
                        .size(self.byte_len)];
                    // SAFETY: barrier and copy reference live buffers owned by
                    // this device and the destination is large enough.
                    unsafe {
                        self.context.device().cmd_pipeline_barrier(
                            command_buffer,
                            vk::PipelineStageFlags::COMPUTE_SHADER
                                | vk::PipelineStageFlags::TRANSFER,
                            vk::PipelineStageFlags::TRANSFER,
                            vk::DependencyFlags::empty(),
                            &[],
                            &source_barrier,
                            &[],
                        );
                        self.context.device().cmd_copy_buffer(
                            command_buffer,
                            self.buffer,
                            staging,
                            &[vk::BufferCopy::default().size(self.byte_len)],
                        );
                    }
                    Ok(())
                },
                VulkanErrorCode::ReadbackFailed,
            )
            .and_then(|()| map_memory(&self.context, memory, self.byte_len, coherent));
        // SAFETY: submit_commands waits for completion before returning.
        unsafe {
            self.context.device().destroy_buffer(staging, None);
            self.context.device().free_memory(memory, None);
        }
        result
    }
}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        // SAFETY: every submission is synchronized before owned resources drop.
        unsafe {
            self.context.device().destroy_buffer(self.buffer, None);
            self.context.device().free_memory(self.memory, None);
        }
    }
}

pub(crate) struct HostBuffer {
    context: Arc<VulkanContext>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    byte_len: vk::DeviceSize,
}

impl HostBuffer {
    pub(crate) fn from_words(
        context: Arc<VulkanContext>,
        words: &[u32],
    ) -> Result<Self, VulkanError> {
        let retained = if words.is_empty() {
            &[0_u32][..]
        } else {
            words
        };
        let byte_len = u64::try_from(retained.len())
            .ok()
            .and_then(|length| length.checked_mul(4))
            .ok_or(VulkanError::new(VulkanErrorCode::UploadFailed))?;
        let (buffer, memory, coherent) = allocate_buffer(
            &context,
            byte_len,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT,
            VulkanErrorCode::UploadFailed,
        )?;
        if let Err(error) = write_memory(&context, memory, retained, coherent) {
            // SAFETY: the buffer was never submitted and both handles are owned here.
            unsafe {
                context.device().destroy_buffer(buffer, None);
                context.device().free_memory(memory, None);
            }
            return Err(error);
        }
        Ok(Self {
            context,
            buffer,
            memory,
            byte_len,
        })
    }

    pub(crate) const fn handle(&self) -> vk::Buffer {
        self.buffer
    }

    pub(crate) const fn byte_len(&self) -> vk::DeviceSize {
        self.byte_len
    }

    pub(crate) fn from_bytes(
        context: Arc<VulkanContext>,
        bytes: &[u8],
    ) -> Result<Self, VulkanError> {
        let retained = if bytes.is_empty() { &[0_u8][..] } else { bytes };
        let byte_len = u64::try_from(retained.len())
            .map_err(|_| VulkanError::new(VulkanErrorCode::UploadFailed))?;
        let (buffer, memory, coherent) = allocate_buffer(
            &context,
            byte_len,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT,
            VulkanErrorCode::UploadFailed,
        )?;
        if let Err(error) = write_bytes(&context, memory, retained, coherent) {
            // SAFETY: the buffer was never submitted and both handles are owned here.
            unsafe {
                context.device().destroy_buffer(buffer, None);
                context.device().free_memory(memory, None);
            }
            return Err(error);
        }
        Ok(Self {
            context,
            buffer,
            memory,
            byte_len,
        })
    }
}

impl Drop for HostBuffer {
    fn drop(&mut self) {
        // SAFETY: dispatch waits before host buffers leave scope.
        unsafe {
            self.context.device().destroy_buffer(self.buffer, None);
            self.context.device().free_memory(self.memory, None);
        }
    }
}

/// One transient RGBA8 image uploaded for a sampled shader invocation.
pub(crate) struct SampledImage {
    context: Arc<VulkanContext>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    sampler: vk::Sampler,
}

impl SampledImage {
    pub(crate) fn transparent(context: Arc<VulkanContext>) -> Result<Self, VulkanError> {
        let image = Image::from_rgba8(1, 1, vec![0, 0, 0, 0])
            .map_err(|_| VulkanError::new(VulkanErrorCode::UploadFailed))?;
        Self::from_image(
            context,
            &image,
            SamplingFilter::Nearest,
            TileMode::Clamp,
            TileMode::Clamp,
        )
    }

    pub(crate) fn from_shader(
        context: Arc<VulkanContext>,
        shader: &ImageShader,
    ) -> Result<Self, VulkanError> {
        Self::from_image(
            context,
            shader.image(),
            shader.sampling().filter(),
            shader.x_tile_mode(),
            shader.y_tile_mode(),
        )
    }

    fn from_image(
        context: Arc<VulkanContext>,
        image: &Image,
        filter: SamplingFilter,
        x_tile_mode: TileMode,
        y_tile_mode: TileMode,
    ) -> Result<Self, VulkanError> {
        let staging = HostBuffer::from_bytes(context.clone(), image.pixels())?;
        let extent = vk::Extent3D {
            width: image.width(),
            height: image.height(),
            depth: 1,
        };
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(extent)
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: info is complete and image dimensions were validated by ImageShader.
        let native_image = unsafe { context.device().create_image(&info, None) }
            .map_err(|_| VulkanError::new(VulkanErrorCode::UploadFailed))?;
        // SAFETY: image belongs to this device and is not in use yet.
        let requirements = unsafe { context.device().get_image_memory_requirements(native_image) };
        let Some((memory_type_index, _)) = context.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::empty(),
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) else {
            // SAFETY: the image is unbound and unused.
            unsafe { context.device().destroy_image(native_image, None) };
            return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
        };
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: allocation follows queried memory requirements.
        let memory = match unsafe { context.device().allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(_) => {
                // SAFETY: image has no bound memory and is unused.
                unsafe { context.device().destroy_image(native_image, None) };
                return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
            }
        };
        // SAFETY: memory matches the image requirements and both handles are owned here.
        if unsafe { context.device().bind_image_memory(native_image, memory, 0) }.is_err() {
            // SAFETY: no submission used either resource.
            unsafe {
                context.device().free_memory(memory, None);
                context.device().destroy_image(native_image, None);
            }
            return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
        }
        let range = color_subresource_range();
        let view_info = vk::ImageViewCreateInfo::default()
            .image(native_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(range);
        // SAFETY: image is bound and the view covers its only color subresource.
        let view = match unsafe { context.device().create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(_) => {
                // SAFETY: no submission used the owned resources.
                unsafe {
                    context.device().free_memory(memory, None);
                    context.device().destroy_image(native_image, None);
                }
                return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
            }
        };
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk_filter(filter))
            .min_filter(vk_filter(filter))
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk_tile_mode(x_tile_mode))
            .address_mode_v(vk_tile_mode(y_tile_mode))
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(0.0);
        // SAFETY: sampler settings contain only supported core Vulkan 1.0 values.
        let sampler = match unsafe { context.device().create_sampler(&sampler_info, None) } {
            Ok(sampler) => sampler,
            Err(_) => {
                // SAFETY: no submission used the owned resources.
                unsafe {
                    context.device().destroy_image_view(view, None);
                    context.device().free_memory(memory, None);
                    context.device().destroy_image(native_image, None);
                }
                return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
            }
        };
        let copy_result = context.submit_commands(
            |command_buffer| {
                let to_transfer = [vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .image(native_image)
                    .subresource_range(range)];
                let layers = vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1);
                let copy = [vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(layers)
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(extent)];
                let to_sampled = [vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ)
                    .image(native_image)
                    .subresource_range(range)];
                // SAFETY: staging, image, and barriers are owned and remain alive through submit.
                unsafe {
                    context.device().cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::TOP_OF_PIPE,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &to_transfer,
                    );
                    context.device().cmd_copy_buffer_to_image(
                        command_buffer,
                        staging.handle(),
                        native_image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &copy,
                    );
                    context.device().cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &to_sampled,
                    );
                }
                Ok(())
            },
            VulkanErrorCode::UploadFailed,
        );
        if let Err(error) = copy_result {
            // SAFETY: submit_commands synchronizes success; failed recording has no pending work.
            unsafe {
                context.device().destroy_sampler(sampler, None);
                context.device().destroy_image_view(view, None);
                context.device().free_memory(memory, None);
                context.device().destroy_image(native_image, None);
            }
            return Err(error);
        }
        Ok(Self {
            context,
            image: native_image,
            memory,
            view,
            sampler,
        })
    }

    pub(crate) const fn view(&self) -> vk::ImageView {
        self.view
    }

    pub(crate) const fn sampler(&self) -> vk::Sampler {
        self.sampler
    }
}

impl Drop for SampledImage {
    fn drop(&mut self) {
        // SAFETY: command submission is synchronous and this object owns every handle.
        unsafe {
            self.context.device().destroy_sampler(self.sampler, None);
            self.context.device().destroy_image_view(self.view, None);
            self.context.device().free_memory(self.memory, None);
            self.context.device().destroy_image(self.image, None);
        }
    }
}

fn color_subresource_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
}

fn vk_filter(filter: SamplingFilter) -> vk::Filter {
    match filter {
        SamplingFilter::Nearest => vk::Filter::NEAREST,
        SamplingFilter::Linear => vk::Filter::LINEAR,
    }
}

fn vk_tile_mode(mode: TileMode) -> vk::SamplerAddressMode {
    match mode {
        TileMode::Clamp => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        TileMode::Repeat => vk::SamplerAddressMode::REPEAT,
        TileMode::Mirror => vk::SamplerAddressMode::MIRRORED_REPEAT,
    }
}

fn allocate_buffer(
    context: &Arc<VulkanContext>,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    required: vk::MemoryPropertyFlags,
    preferred: vk::MemoryPropertyFlags,
    error: VulkanErrorCode,
) -> Result<(vk::Buffer, vk::DeviceMemory, bool), VulkanError> {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    // SAFETY: info is complete and the device is valid.
    let buffer = unsafe { context.device().create_buffer(&info, None) }
        .map_err(|_| VulkanError::new(error))?;
    // SAFETY: buffer belongs to this device.
    let requirements = unsafe { context.device().get_buffer_memory_requirements(buffer) };
    let Some((memory_type_index, flags)) =
        context.memory_type(requirements.memory_type_bits, required, preferred)
    else {
        // SAFETY: unbound buffer is unused.
        unsafe { context.device().destroy_buffer(buffer, None) };
        return Err(VulkanError::new(error));
    };
    let allocation = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    // SAFETY: allocation follows queried requirements.
    let memory = match unsafe { context.device().allocate_memory(&allocation, None) } {
        Ok(memory) => memory,
        Err(_) => {
            // SAFETY: unbound buffer is unused.
            unsafe { context.device().destroy_buffer(buffer, None) };
            return Err(VulkanError::new(error));
        }
    };
    // SAFETY: memory meets buffer requirements at offset zero.
    if unsafe { context.device().bind_buffer_memory(buffer, memory, 0) }.is_err() {
        // SAFETY: neither handle is in use.
        unsafe {
            context.device().free_memory(memory, None);
            context.device().destroy_buffer(buffer, None);
        }
        return Err(VulkanError::new(error));
    }
    Ok((
        buffer,
        memory,
        flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT),
    ))
}

fn write_memory(
    context: &Arc<VulkanContext>,
    memory: vk::DeviceMemory,
    words: &[u32],
    coherent: bool,
) -> Result<(), VulkanError> {
    // SAFETY: allocation is HOST_VISIBLE and large enough for words.
    let mapped = unsafe {
        context
            .device()
            .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
    }
    .map_err(|_| VulkanError::new(VulkanErrorCode::UploadFailed))?;
    // SAFETY: mapped range contains words.len() u32 values.
    unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), mapped.cast(), words.len()) };
    if !coherent {
        let ranges = [vk::MappedMemoryRange::default()
            .memory(memory)
            .offset(0)
            .size(vk::WHOLE_SIZE)];
        // SAFETY: allocation is currently mapped.
        if unsafe { context.device().flush_mapped_memory_ranges(&ranges) }.is_err() {
            // SAFETY: mapping is active.
            unsafe { context.device().unmap_memory(memory) };
            return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
        }
    }
    // SAFETY: mapping is active and writes are finished.
    unsafe { context.device().unmap_memory(memory) };
    Ok(())
}

fn write_bytes(
    context: &Arc<VulkanContext>,
    memory: vk::DeviceMemory,
    bytes: &[u8],
    coherent: bool,
) -> Result<(), VulkanError> {
    // SAFETY: allocation is HOST_VISIBLE and large enough for bytes.
    let mapped = unsafe {
        context
            .device()
            .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
    }
    .map_err(|_| VulkanError::new(VulkanErrorCode::UploadFailed))?;
    // SAFETY: mapped range contains bytes.len() bytes.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast(), bytes.len()) };
    if !coherent {
        let ranges = [vk::MappedMemoryRange::default()
            .memory(memory)
            .offset(0)
            .size(vk::WHOLE_SIZE)];
        // SAFETY: allocation is currently mapped.
        if unsafe { context.device().flush_mapped_memory_ranges(&ranges) }.is_err() {
            // SAFETY: mapping is active.
            unsafe { context.device().unmap_memory(memory) };
            return Err(VulkanError::new(VulkanErrorCode::UploadFailed));
        }
    }
    // SAFETY: mapping is active and writes are finished.
    unsafe { context.device().unmap_memory(memory) };
    Ok(())
}

fn map_memory(
    context: &Arc<VulkanContext>,
    memory: vk::DeviceMemory,
    byte_len: vk::DeviceSize,
    coherent: bool,
) -> Result<Vec<u8>, VulkanError> {
    // SAFETY: allocation is HOST_VISIBLE and queue work has completed.
    let mapped = unsafe {
        context
            .device()
            .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
    }
    .map_err(|_| VulkanError::new(VulkanErrorCode::ReadbackFailed))?;
    if !coherent {
        let ranges = [vk::MappedMemoryRange::default()
            .memory(memory)
            .offset(0)
            .size(vk::WHOLE_SIZE)];
        // SAFETY: allocation is currently mapped.
        if unsafe { context.device().invalidate_mapped_memory_ranges(&ranges) }.is_err() {
            // SAFETY: mapping is active.
            unsafe { context.device().unmap_memory(memory) };
            return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
        }
    }
    let length =
        usize::try_from(byte_len).map_err(|_| VulkanError::new(VulkanErrorCode::ReadbackFailed))?;
    let bytes = unsafe { std::slice::from_raw_parts(mapped.cast::<u8>(), length) };
    let output = bytes.to_vec();
    // SAFETY: mapping is active.
    unsafe { context.device().unmap_memory(memory) };
    Ok(output)
}

fn pixel_byte_length(
    descriptor: GpuSurfaceDescriptor,
    error: VulkanErrorCode,
) -> Result<vk::DeviceSize, VulkanError> {
    u64::from(descriptor.width())
        .checked_mul(u64::from(descriptor.height()))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(VulkanError::new(error))
}
