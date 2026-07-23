use std::{io::Cursor, sync::Arc};

use ash::vk;

use crate::{
    VulkanError, VulkanErrorCode,
    context::VulkanContext,
    surface::{HostBuffer, PixelBuffer},
};

pub(crate) struct VulkanRenderer {
    context: Arc<VulkanContext>,
    descriptor_set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    dummy: PixelBuffer,
}

impl VulkanRenderer {
    pub(crate) fn new(context: Arc<VulkanContext>) -> Result<Self, VulkanError> {
        let dummy_descriptor = skia_gpu::GpuSurfaceDescriptor::new(1, 1)
            .map_err(|_| VulkanError::new(VulkanErrorCode::SurfaceAllocationFailed))?;
        let dummy = PixelBuffer::new(context.clone(), dummy_descriptor)?;
        dummy.clear(skia_core::Color::TRANSPARENT)?;
        let bindings = (0..5)
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE)
            })
            .collect::<Vec<_>>();
        let descriptor_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        // SAFETY: descriptor bindings are complete and retained through the call.
        let descriptor_set_layout = unsafe {
            context
                .device()
                .create_descriptor_set_layout(&descriptor_info, None)
        }
        .map_err(|_| VulkanError::new(VulkanErrorCode::PipelineCreationFailed))?;
        let set_layouts = [descriptor_set_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        // SAFETY: descriptor set layout belongs to this device.
        let pipeline_layout =
            match unsafe { context.device().create_pipeline_layout(&layout_info, None) } {
                Ok(layout) => layout,
                Err(_) => {
                    // SAFETY: layout is unused.
                    unsafe {
                        context
                            .device()
                            .destroy_descriptor_set_layout(descriptor_set_layout, None)
                    };
                    return Err(VulkanError::new(VulkanErrorCode::PipelineCreationFailed));
                }
            };
        let shader_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/renderer.spv"));
        let words = match ash::util::read_spv(&mut Cursor::new(shader_bytes.as_slice())) {
            Ok(words) => words,
            Err(_) => {
                destroy_layouts(&context, pipeline_layout, descriptor_set_layout);
                return Err(VulkanError::new(VulkanErrorCode::ShaderModuleFailed));
            }
        };
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&words);
        // SAFETY: SPIR-V was generated and validated by the build script.
        let shader = match unsafe { context.device().create_shader_module(&shader_info, None) } {
            Ok(shader) => shader,
            Err(_) => {
                // SAFETY: layouts are not in use.
                destroy_layouts(&context, pipeline_layout, descriptor_set_layout);
                return Err(VulkanError::new(VulkanErrorCode::ShaderModuleFailed));
            }
        };
        let entry = c"main";
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(shader)
            .name(entry);
        let pipeline_info = [vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(pipeline_layout)];
        // SAFETY: shader module and pipeline layout are valid for this device.
        let pipeline_result = unsafe {
            context.device().create_compute_pipelines(
                vk::PipelineCache::null(),
                &pipeline_info,
                None,
            )
        };
        // SAFETY: pipeline creation no longer needs the shader module.
        unsafe { context.device().destroy_shader_module(shader, None) };
        let pipeline = match pipeline_result {
            Ok(mut pipelines) => match pipelines.pop() {
                Some(pipeline) => pipeline,
                None => {
                    destroy_layouts(&context, pipeline_layout, descriptor_set_layout);
                    return Err(VulkanError::new(VulkanErrorCode::PipelineCreationFailed));
                }
            },
            Err((pipelines, _)) => {
                // SAFETY: failed batch pipelines, if any, are not submitted or retained.
                unsafe {
                    for pipeline in pipelines {
                        context.device().destroy_pipeline(pipeline, None);
                    }
                }
                destroy_layouts(&context, pipeline_layout, descriptor_set_layout);
                return Err(VulkanError::new(VulkanErrorCode::PipelineCreationFailed));
            }
        };
        Ok(Self {
            context,
            descriptor_set_layout,
            pipeline_layout,
            pipeline,
            dummy,
        })
    }

    pub(crate) fn dispatch(
        &self,
        output: &PixelBuffer,
        source: Option<&PixelBuffer>,
        clip: Option<&PixelBuffer>,
        payload_words: &[u32],
        parameter_words: &[u32],
    ) -> Result<(), VulkanError> {
        let payload = HostBuffer::from_words(self.context.clone(), payload_words)?;
        let parameters = HostBuffer::from_words(self.context.clone(), parameter_words)?;
        let source = source.unwrap_or(&self.dummy);
        let clip = clip.unwrap_or(&self.dummy);
        let pool_sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(5)];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        // SAFETY: pool configuration is complete.
        let pool = unsafe {
            self.context
                .device()
                .create_descriptor_pool(&pool_info, None)
        }
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
        let layouts = [self.descriptor_set_layout];
        let set_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        // SAFETY: pool and layout belong to this device.
        let set = match unsafe { self.context.device().allocate_descriptor_sets(&set_info) } {
            Ok(sets) => sets[0],
            Err(_) => {
                // SAFETY: pool has no live allocated set.
                unsafe { self.context.device().destroy_descriptor_pool(pool, None) };
                return Err(VulkanError::new(VulkanErrorCode::SubmissionFailed));
            }
        };
        let infos = [
            [buffer_info(output.handle(), output.byte_len())],
            [buffer_info(source.handle(), source.byte_len())],
            [buffer_info(payload.handle(), payload.byte_len())],
            [buffer_info(parameters.handle(), parameters.byte_len())],
            [buffer_info(clip.handle(), clip.byte_len())],
        ];
        let writes = infos
            .iter()
            .enumerate()
            .map(|(binding, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(u32::try_from(binding).unwrap_or(0))
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(info)
            })
            .collect::<Vec<_>>();
        // SAFETY: descriptors point at buffers alive through submission.
        unsafe { self.context.device().update_descriptor_sets(&writes, &[]) };
        let width = output.descriptor().width().div_ceil(8);
        let height = output.descriptor().height().div_ceil(8);
        let result = self.context.submit_commands(
            |command_buffer| {
                let mut barriers = vec![
                    vk::BufferMemoryBarrier::default()
                        .src_access_mask(
                            vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE,
                        )
                        .dst_access_mask(
                            vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                        )
                        .buffer(output.handle())
                        .offset(0)
                        .size(output.byte_len()),
                ];
                if source.handle() != output.handle() {
                    barriers.push(
                        vk::BufferMemoryBarrier::default()
                            .src_access_mask(
                                vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE,
                            )
                            .dst_access_mask(vk::AccessFlags::SHADER_READ)
                            .buffer(source.handle())
                            .offset(0)
                            .size(source.byte_len()),
                    );
                }
                if clip.handle() != output.handle() && clip.handle() != source.handle() {
                    barriers.push(
                        vk::BufferMemoryBarrier::default()
                            .src_access_mask(
                                vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE,
                            )
                            .dst_access_mask(vk::AccessFlags::SHADER_READ)
                            .buffer(clip.handle())
                            .offset(0)
                            .size(clip.byte_len()),
                    );
                }
                // SAFETY: pipeline, set, and buffers belong to the recording device.
                unsafe {
                    self.context.device().cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &barriers,
                        &[],
                    );
                    self.context.device().cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::COMPUTE,
                        self.pipeline,
                    );
                    self.context.device().cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::COMPUTE,
                        self.pipeline_layout,
                        0,
                        &[set],
                        &[],
                    );
                    self.context
                        .device()
                        .cmd_dispatch(command_buffer, width, height, 1);
                }
                Ok(())
            },
            VulkanErrorCode::SubmissionFailed,
        );
        // SAFETY: submit_commands waits for all descriptor users to finish.
        unsafe { self.context.device().destroy_descriptor_pool(pool, None) };
        result
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        // SAFETY: submissions are synchronous and handles are owned here.
        unsafe {
            self.context.device().destroy_pipeline(self.pipeline, None);
            self.context
                .device()
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.context
                .device()
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        }
    }
}

fn buffer_info(buffer: vk::Buffer, range: vk::DeviceSize) -> vk::DescriptorBufferInfo {
    vk::DescriptorBufferInfo::default()
        .buffer(buffer)
        .offset(0)
        .range(range)
}

fn destroy_layouts(
    context: &VulkanContext,
    pipeline_layout: vk::PipelineLayout,
    descriptor_set_layout: vk::DescriptorSetLayout,
) {
    // SAFETY: callers invoke this only before either layout is used by a pipeline.
    unsafe {
        context
            .device()
            .destroy_pipeline_layout(pipeline_layout, None);
        context
            .device()
            .destroy_descriptor_set_layout(descriptor_set_layout, None);
    }
}
