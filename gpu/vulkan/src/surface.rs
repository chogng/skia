use std::sync::Arc;

use ash::vk;
use skia_core::Color;
use skia_gpu::GpuSurfaceDescriptor;

use crate::{VulkanError, VulkanErrorCode, context::VulkanContext};

/// Vulkan-owned offscreen RGBA8 image.
pub struct VulkanSurface {
    context: Arc<VulkanContext>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    descriptor: GpuSurfaceDescriptor,
    initialized: bool,
}

impl VulkanSurface {
    pub(crate) fn new(
        context: Arc<VulkanContext>,
        descriptor: GpuSurfaceDescriptor,
    ) -> Result<Self, VulkanError> {
        let format = vk::Format::R8G8B8A8_UNORM;
        // SAFETY: physical_device belongs to context.instance for its full lifetime.
        let format_properties = unsafe {
            context
                .instance()
                .get_physical_device_format_properties(context.physical_device(), format)
        };
        let required_features =
            vk::FormatFeatureFlags::TRANSFER_SRC | vk::FormatFeatureFlags::TRANSFER_DST;
        if !format_properties
            .optimal_tiling_features
            .contains(required_features)
        {
            return Err(VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed));
        }
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: descriptor.width(),
                height: descriptor.height(),
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::COLOR_ATTACHMENT,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: image_info is fully initialized and context device is valid.
        let image = unsafe { context.device().create_image(&image_info, None) }
            .map_err(|_| VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed))?;
        // SAFETY: image belongs to context device.
        let requirements = unsafe { context.device().get_image_memory_requirements(image) };
        let Some((memory_type_index, _)) = context.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::empty(),
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) else {
            // SAFETY: image was created above and is not bound or in use.
            unsafe { context.device().destroy_image(image, None) };
            return Err(VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed));
        };
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: allocation uses requirements from this device image.
        let memory = match unsafe { context.device().allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(_) => {
                // SAFETY: image is not bound or in use.
                unsafe { context.device().destroy_image(image, None) };
                return Err(VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed));
            }
        };
        // SAFETY: memory satisfies image requirements and offset zero alignment.
        if unsafe { context.device().bind_image_memory(image, memory, 0) }.is_err() {
            // SAFETY: neither handle is in use and both belong to the device.
            unsafe {
                context.device().free_memory(memory, None);
                context.device().destroy_image(image, None);
            }
            return Err(VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed));
        }
        Ok(Self {
            context,
            image,
            memory,
            descriptor,
            initialized: false,
        })
    }

    /// Returns the portable descriptor used to allocate the image.
    pub const fn descriptor(&self) -> GpuSurfaceDescriptor {
        self.descriptor
    }

    /// Reads the completed image as tightly packed row-major straight RGBA8.
    pub fn read_rgba8(&self) -> Result<Vec<u8>, VulkanError> {
        if !self.initialized {
            return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
        }
        let length = image_byte_length(self.descriptor)?;
        let (buffer, memory, coherent) = self.create_readback_buffer(length)?;
        let copy_result = self.context.submit_commands(
            |command_buffer| {
                transition_image(
                    self.context.device(),
                    command_buffer,
                    self.image,
                    ImageTransition {
                        old_layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        new_layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        src_access: vk::AccessFlags::TRANSFER_WRITE,
                        dst_access: vk::AccessFlags::TRANSFER_READ,
                        src_stage: vk::PipelineStageFlags::TRANSFER,
                        dst_stage: vk::PipelineStageFlags::TRANSFER,
                    },
                );
                let region = [vk::BufferImageCopy::default()
                    .image_subresource(color_subresource())
                    .image_extent(vk::Extent3D {
                        width: self.descriptor.width(),
                        height: self.descriptor.height(),
                        depth: 1,
                    })];
                // SAFETY: source image and destination buffer have compatible
                // transfer usage, valid layouts, and sufficient allocation.
                unsafe {
                    self.context.device().cmd_copy_image_to_buffer(
                        command_buffer,
                        self.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        buffer,
                        &region,
                    )
                };
                let buffer_barrier = [vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)
                    .buffer(buffer)
                    .offset(0)
                    .size(vk::WHOLE_SIZE)];
                // SAFETY: the barrier refers to the just-recorded buffer copy.
                unsafe {
                    self.context.device().cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::HOST,
                        vk::DependencyFlags::empty(),
                        &[],
                        &buffer_barrier,
                        &[],
                    )
                };
                transition_image(
                    self.context.device(),
                    command_buffer,
                    self.image,
                    ImageTransition {
                        old_layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        new_layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        src_access: vk::AccessFlags::TRANSFER_READ,
                        dst_access: vk::AccessFlags::TRANSFER_WRITE,
                        src_stage: vk::PipelineStageFlags::TRANSFER,
                        dst_stage: vk::PipelineStageFlags::TRANSFER,
                    },
                );
                Ok(())
            },
            VulkanErrorCode::ReadbackFailed,
        );
        let output = copy_result.and_then(|()| self.map_readback(memory, length, coherent));
        // SAFETY: submit_commands waits for completion, so the staging handles
        // are not pending even when mapping fails.
        unsafe {
            self.context.device().destroy_buffer(buffer, None);
            self.context.device().free_memory(memory, None);
        }
        output
    }

    pub(crate) fn clear(&mut self, colors: impl Iterator<Item = Color>) -> Result<(), VulkanError> {
        let Some(color) = colors.last() else {
            return Ok(());
        };
        let old_layout = if self.initialized {
            vk::ImageLayout::TRANSFER_DST_OPTIMAL
        } else {
            vk::ImageLayout::UNDEFINED
        };
        let old_access = if self.initialized {
            vk::AccessFlags::TRANSFER_WRITE
        } else {
            vk::AccessFlags::empty()
        };
        let old_stage = if self.initialized {
            vk::PipelineStageFlags::TRANSFER
        } else {
            vk::PipelineStageFlags::TOP_OF_PIPE
        };
        self.context.submit_commands(
            |command_buffer| {
                if !self.initialized {
                    transition_image(
                        self.context.device(),
                        command_buffer,
                        self.image,
                        ImageTransition {
                            old_layout,
                            new_layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            src_access: old_access,
                            dst_access: vk::AccessFlags::TRANSFER_WRITE,
                            src_stage: old_stage,
                            dst_stage: vk::PipelineStageFlags::TRANSFER,
                        },
                    );
                }
                let [red, green, blue, alpha] = color.channels();
                let scale = f32::from(u8::MAX);
                let clear = vk::ClearColorValue {
                    float32: [
                        f32::from(red) / scale,
                        f32::from(green) / scale,
                        f32::from(blue) / scale,
                        f32::from(alpha) / scale,
                    ],
                };
                let range = [color_range()];
                // SAFETY: image is in TRANSFER_DST_OPTIMAL with color aspect.
                unsafe {
                    self.context.device().cmd_clear_color_image(
                        command_buffer,
                        self.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &clear,
                        &range,
                    )
                };
                Ok(())
            },
            VulkanErrorCode::SubmissionFailed,
        )?;
        self.initialized = true;
        Ok(())
    }

    fn create_readback_buffer(
        &self,
        length: usize,
    ) -> Result<(vk::Buffer, vk::DeviceMemory, bool), VulkanError> {
        let size =
            u64::try_from(length).map_err(|_| VulkanError::new(VulkanErrorCode::ReadbackFailed))?;
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: buffer_info is complete and device is valid.
        let buffer = unsafe { self.context.device().create_buffer(&buffer_info, None) }
            .map_err(|_| VulkanError::new(VulkanErrorCode::ReadbackFailed))?;
        // SAFETY: buffer belongs to context device.
        let requirements = unsafe { self.context.device().get_buffer_memory_requirements(buffer) };
        let Some((memory_type_index, flags)) = self.context.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT | vk::MemoryPropertyFlags::HOST_CACHED,
        ) else {
            // SAFETY: buffer is unbound and unused.
            unsafe { self.context.device().destroy_buffer(buffer, None) };
            return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
        };
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: allocation uses this buffer's requirements.
        let memory = match unsafe { self.context.device().allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(_) => {
                // SAFETY: buffer is unbound and unused.
                unsafe { self.context.device().destroy_buffer(buffer, None) };
                return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
            }
        };
        // SAFETY: memory satisfies the buffer requirements at offset zero.
        if unsafe { self.context.device().bind_buffer_memory(buffer, memory, 0) }.is_err() {
            // SAFETY: neither handle is in use.
            unsafe {
                self.context.device().free_memory(memory, None);
                self.context.device().destroy_buffer(buffer, None);
            }
            return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
        }
        Ok((
            buffer,
            memory,
            flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT),
        ))
    }

    fn map_readback(
        &self,
        memory: vk::DeviceMemory,
        length: usize,
        coherent: bool,
    ) -> Result<Vec<u8>, VulkanError> {
        // SAFETY: memory is HOST_VISIBLE; mapping the whole allocation also
        // permits a whole-range invalidate on non-coherent heaps.
        let mapped = unsafe {
            self.context
                .device()
                .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
        }
        .map_err(|_| VulkanError::new(VulkanErrorCode::ReadbackFailed))?;
        if !coherent {
            let range = [vk::MappedMemoryRange::default()
                .memory(memory)
                .offset(0)
                .size(vk::WHOLE_SIZE)];
            // SAFETY: queue work has completed and range covers this allocation.
            if unsafe {
                self.context
                    .device()
                    .invalidate_mapped_memory_ranges(&range)
            }
            .is_err()
            {
                // SAFETY: mapped was returned by map_memory above.
                unsafe { self.context.device().unmap_memory(memory) };
                return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
            }
        }
        let mut output = Vec::new();
        if output.try_reserve_exact(length).is_err() {
            // SAFETY: mapped was returned by map_memory and is still mapped.
            unsafe { self.context.device().unmap_memory(memory) };
            return Err(VulkanError::new(VulkanErrorCode::ReadbackFailed));
        }
        // SAFETY: the mapped range contains at least length bytes and remains
        // valid until unmap_memory below.
        let bytes = unsafe { std::slice::from_raw_parts(mapped.cast::<u8>(), length) };
        output.extend_from_slice(bytes);
        // SAFETY: mapped was returned by map_memory and has not been unmapped.
        unsafe { self.context.device().unmap_memory(memory) };
        Ok(output)
    }
}

impl Drop for VulkanSurface {
    fn drop(&mut self) {
        // SAFETY: every submission waits for completion, and the surface owns
        // both handles exclusively until this drop.
        unsafe {
            self.context.device().destroy_image(self.image, None);
            self.context.device().free_memory(self.memory, None);
        }
    }
}

#[derive(Clone, Copy)]
struct ImageTransition {
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
}

fn transition_image(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    image: vk::Image,
    transition: ImageTransition,
) {
    let barriers = [vk::ImageMemoryBarrier::default()
        .src_access_mask(transition.src_access)
        .dst_access_mask(transition.dst_access)
        .old_layout(transition.old_layout)
        .new_layout(transition.new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_range())];
    // SAFETY: barrier references the supplied image and records into an active
    // primary command buffer owned by the same device.
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            transition.src_stage,
            transition.dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &barriers,
        )
    };
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
}

fn color_subresource() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .mip_level(0)
        .base_array_layer(0)
        .layer_count(1)
}

fn image_byte_length(descriptor: GpuSurfaceDescriptor) -> Result<usize, VulkanError> {
    u64::from(descriptor.width())
        .checked_mul(u64::from(descriptor.height()))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(VulkanError::new(VulkanErrorCode::ReadbackFailed))
}
