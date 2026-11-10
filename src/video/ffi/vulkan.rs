//! Ash types under the names the generated FFmpeg and libplacebo bindings use
//!
//! Ash's handles, flags, and structures share the C ABI of the Vulkan headers

#![allow(non_camel_case_types)]

use ash::vk;

pub type VkInstance = vk::Instance;
pub type VkPhysicalDevice = vk::PhysicalDevice;
pub type VkDevice = vk::Device;
pub type VkImage = vk::Image;
pub type VkSemaphore = vk::Semaphore;
pub type VkDeviceMemory = vk::DeviceMemory;
pub type VkSurfaceKHR = vk::SurfaceKHR;

pub type VkFormat = vk::Format;
pub type VkImageLayout = vk::ImageLayout;
pub type VkImageTiling = vk::ImageTiling;

pub type VkAccessFlagBits2 = vk::AccessFlags2;
pub type VkDeviceQueueCreateFlags = vk::DeviceQueueCreateFlags;
pub type VkImageAspectFlags = vk::ImageAspectFlags;
pub type VkImageCreateFlags = vk::ImageCreateFlags;
pub type VkImageUsageFlagBits = vk::ImageUsageFlags;
pub type VkImageUsageFlags = vk::ImageUsageFlags;
pub type VkMemoryPropertyFlagBits = vk::MemoryPropertyFlags;
pub type VkQueueFlagBits = vk::QueueFlags;
pub type VkQueueFlags = vk::QueueFlags;
pub type VkVideoCodecOperationFlagBitsKHR = vk::VideoCodecOperationFlagsKHR;

pub type VkAllocationCallbacks = vk::AllocationCallbacks<'static>;
pub type VkPhysicalDeviceFeatures2 = vk::PhysicalDeviceFeatures2<'static>;

/// C leaves this pointer unset in zeroed structures, which ash's type cannot express
pub type PFN_vkGetInstanceProcAddr = Option<vk::PFN_vkGetInstanceProcAddr>;
