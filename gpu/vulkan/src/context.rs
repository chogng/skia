use std::{
    ffi::{CStr, CString},
    sync::Mutex,
};

use ash::{Entry, vk};

use crate::{VulkanError, VulkanErrorCode};

pub(crate) struct VulkanContext {
    _entry: Entry,
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    queue_family_index: u32,
    command_pool: vk::CommandPool,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    device_name: String,
    validation_enabled: bool,
    queue_lock: Mutex<()>,
}

impl VulkanContext {
    pub(crate) fn new() -> Result<Self, VulkanError> {
        // SAFETY: ash retains the loader library for the lifetime of Entry. The
        // entry is stored in VulkanContext and outlives every derived handle.
        let entry = unsafe { Entry::load() }
            .map_err(|_| VulkanError::new(VulkanErrorCode::LoaderUnavailable))?;
        let application_name = c"skia-vulkan";
        let application = vk::ApplicationInfo::default()
            .application_name(application_name)
            .application_version(1)
            .engine_name(application_name)
            .engine_version(1)
            .api_version(vk::API_VERSION_1_0);
        let validation_enabled = std::env::var_os("SKIA_VULKAN_VALIDATION").is_some();
        let validation_layer = CString::new("VK_LAYER_KHRONOS_validation")
            .map_err(|_| VulkanError::new(VulkanErrorCode::InstanceCreationFailed))?;
        let layer_names = if validation_enabled {
            // SAFETY: entry is live and Vulkan returns fixed-size layer records.
            let layers = unsafe { entry.enumerate_instance_layer_properties() }
                .map_err(|_| VulkanError::new(VulkanErrorCode::InstanceCreationFailed))?;
            let available = layers.iter().any(|layer| {
                // SAFETY: Vulkan guarantees layer_name is nul-terminated.
                (unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) })
                    == validation_layer.as_c_str()
            });
            if !available {
                return Err(VulkanError::new(VulkanErrorCode::ValidationUnavailable));
            }
            vec![validation_layer.as_ptr()]
        } else {
            Vec::new()
        };
        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_layer_names(&layer_names);
        // SAFETY: instance_info only references application until the call returns;
        // no extensions, layers, or custom allocators are supplied.
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|_| VulkanError::new(VulkanErrorCode::InstanceCreationFailed))?;
        let selected = select_device(&instance);
        let (physical_device, queue_family_index, device_name) = match selected {
            Ok(selected) => selected,
            Err(error) => {
                // SAFETY: instance was created above and has no child device.
                unsafe { instance.destroy_instance(None) };
                return Err(error);
            }
        };
        let priorities = [1.0_f32];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];
        let device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queue_info);
        // SAFETY: the selected physical device and queue family belong to instance.
        let device = match unsafe { instance.create_device(physical_device, &device_info, None) } {
            Ok(device) => device,
            Err(_) => {
                // SAFETY: instance has no successfully created child device.
                unsafe { instance.destroy_instance(None) };
                return Err(VulkanError::new(VulkanErrorCode::DeviceCreationFailed));
            }
        };
        // SAFETY: one queue was requested from this family at device creation.
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        // SAFETY: pool_info references a valid queue family on device.
        let command_pool = match unsafe { device.create_command_pool(&pool_info, None) } {
            Ok(pool) => pool,
            Err(_) => {
                // SAFETY: the device is idle because no work has been submitted.
                unsafe {
                    device.destroy_device(None);
                    instance.destroy_instance(None);
                }
                return Err(VulkanError::new(VulkanErrorCode::DeviceCreationFailed));
            }
        };
        // SAFETY: physical_device belongs to instance and remains valid.
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };
        Ok(Self {
            _entry: entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
            command_pool,
            memory_properties,
            device_name,
            validation_enabled,
            queue_lock: Mutex::new(()),
        })
    }

    pub(crate) const fn device(&self) -> &ash::Device {
        &self.device
    }

    pub(crate) const fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    pub(crate) const fn instance(&self) -> &ash::Instance {
        &self.instance
    }

    pub(crate) fn device_name(&self) -> String {
        self.device_name.clone()
    }

    pub(crate) const fn queue_family_index(&self) -> u32 {
        self.queue_family_index
    }

    pub(crate) const fn validation_enabled(&self) -> bool {
        self.validation_enabled
    }

    pub(crate) fn memory_type(
        &self,
        supported_bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> Option<(u32, vk::MemoryPropertyFlags)> {
        let count = usize::try_from(self.memory_properties.memory_type_count).ok()?;
        let candidates = self.memory_properties.memory_types.get(..count)?;
        candidates
            .iter()
            .enumerate()
            .filter(|(index, memory_type)| {
                supported_bits & (1_u32 << index) != 0
                    && memory_type.property_flags.contains(required)
            })
            .max_by_key(|(_, memory_type)| {
                (memory_type.property_flags & preferred)
                    .as_raw()
                    .count_ones()
            })
            .and_then(|(index, memory_type)| {
                u32::try_from(index)
                    .ok()
                    .map(|index| (index, memory_type.property_flags))
            })
    }

    pub(crate) fn submit_commands(
        &self,
        record: impl FnOnce(vk::CommandBuffer) -> Result<(), VulkanError>,
        error_code: VulkanErrorCode,
    ) -> Result<(), VulkanError> {
        let _guard = self
            .queue_lock
            .lock()
            .map_err(|_| VulkanError::new(error_code))?;
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: command_pool belongs to device and access is externally synchronized.
        let command_buffer = unsafe { self.device.allocate_command_buffers(&allocate) }
            .map_err(|_| VulkanError::new(error_code))?
            .into_iter()
            .next()
            .ok_or(VulkanError::new(error_code))?;
        let result = (|| {
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            // SAFETY: command_buffer is newly allocated and not pending.
            unsafe { self.device.begin_command_buffer(command_buffer, &begin) }
                .map_err(|_| VulkanError::new(error_code))?;
            record(command_buffer)?;
            // SAFETY: recording was begun and the callback returned successfully.
            unsafe { self.device.end_command_buffer(command_buffer) }
                .map_err(|_| VulkanError::new(error_code))?;
            let command_buffers = [command_buffer];
            let submit = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
            let fence_info = vk::FenceCreateInfo::default();
            // SAFETY: fence_info has no pointer members and device is valid.
            let fence = unsafe { self.device.create_fence(&fence_info, None) }
                .map_err(|_| VulkanError::new(error_code))?;
            // SAFETY: queue, command buffer, and fence belong to device and are synchronized.
            let submitted = unsafe { self.device.queue_submit(self.queue, &submit, fence) };
            let waited = submitted
                .and_then(|()| unsafe { self.device.wait_for_fences(&[fence], true, u64::MAX) });
            // SAFETY: queue work has completed or submission failed; fence is no longer used.
            unsafe { self.device.destroy_fence(fence, None) };
            waited.map_err(|_| VulkanError::new(error_code))
        })();
        // SAFETY: the queue was waited before success; on recording/submission
        // failure Vulkan permits freeing a non-pending command buffer.
        unsafe {
            self.device
                .free_command_buffers(self.command_pool, &[command_buffer])
        };
        result
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        let _guard = self.queue_lock.lock().ok();
        // SAFETY: VulkanContext owns these handles and drops after all Arc-held
        // surfaces. Waiting makes command-pool and device destruction ordered.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn select_device(
    instance: &ash::Instance,
) -> Result<(vk::PhysicalDevice, u32, String), VulkanError> {
    // SAFETY: instance is valid and enumeration writes into ash-owned storage.
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|_| VulkanError::new(VulkanErrorCode::DeviceUnavailable))?;
    devices
        .into_iter()
        .filter_map(|physical_device| {
            // SAFETY: physical_device was returned by this instance.
            let queues =
                unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
            let queue_family = queues
                .iter()
                .position(|queue| {
                    queue.queue_count > 0 && queue.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                })
                .and_then(|index| u32::try_from(index).ok())?;
            // SAFETY: physical_device was returned by this instance.
            let properties = unsafe { instance.get_physical_device_properties(physical_device) };
            let score = match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 4,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
                vk::PhysicalDeviceType::CPU => 1,
                _ => 0,
            };
            // SAFETY: Vulkan guarantees device_name is a nul-terminated array.
            let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            Some((score, physical_device, queue_family, name))
        })
        .max_by_key(|(score, _, _, _)| *score)
        .map(|(_, device, queue, name)| (device, queue, name))
        .ok_or(VulkanError::new(VulkanErrorCode::DeviceUnavailable))
}
