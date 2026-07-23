use std::sync::Arc;

use ash::vk;
use skia_core::Color;
use skia_gpu::GpuSurfaceDescriptor;

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
            vk::BufferUsageFlags::STORAGE_BUFFER,
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
