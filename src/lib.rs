#![doc = include_str!("../README.md")]

mod backend;
mod device;
mod device_info;
mod error;
mod traits;
mod utils;
pub(crate) mod maybe_send;
pub(crate) use maybe_send::{MaybeSend, MaybeSync};

/// All available backends for the current platform
pub use backend::BackendType;
pub use device::{DeviceFeatureHandle, DeviceReader, DeviceReaderWriter, DeviceWriter};
pub use device_info::{Device, DeviceEvent, DeviceId, DeviceInfo, HidBackend};
#[cfg(not(target_arch = "wasm32"))]
use static_assertions::assert_impl_all;
pub use traits::{AsyncHidFeatureHandle, AsyncHidRead, AsyncHidWrite};

pub use crate::error::{HidError, HidResult};

#[cfg(target_arch = "wasm32")]
pub use backend::{request_device, RequestFilter};

#[cfg(not(target_arch = "wasm32"))]
assert_impl_all!(DeviceReaderWriter: Send, Sync);
#[cfg(not(target_arch = "wasm32"))]
assert_impl_all!(DeviceInfo: Send, Sync);
