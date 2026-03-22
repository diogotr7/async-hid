use std::fmt::Debug;
use std::future::Future;
use std::hash::Hash;

use crate::device_info::DeviceId;
use crate::traits::{AsyncHidFeatureHandle, AsyncHidRead, AsyncHidWrite};
use crate::{DeviceEvent, DeviceInfo, HidResult, MaybeSend, MaybeSync};

#[cfg(not(target_arch = "wasm32"))]
pub type DeviceInfoStream = futures_lite::stream::Boxed<HidResult<DeviceInfo>>;
#[cfg(target_arch = "wasm32")]
pub type DeviceInfoStream = std::pin::Pin<Box<dyn futures_lite::Stream<Item = HidResult<DeviceInfo>> + 'static>>;

#[cfg(not(target_arch = "wasm32"))]
pub type EventStream = futures_lite::stream::Boxed<DeviceEvent>;
#[cfg(target_arch = "wasm32")]
pub type EventStream = std::pin::Pin<Box<dyn futures_lite::Stream<Item = DeviceEvent> + 'static>>;

pub trait Backend: Sized + Default {
    type Reader: AsyncHidRead + MaybeSend + MaybeSync;
    type Writer: AsyncHidWrite + MaybeSend + MaybeSync;
    type FeatureHandle: AsyncHidFeatureHandle + MaybeSend + MaybeSync;

    fn enumerate(&self) -> impl Future<Output = HidResult<DeviceInfoStream>> + MaybeSend;
    fn watch(&self) -> HidResult<EventStream>;

    fn query_info(&self, id: &DeviceId) -> impl Future<Output = HidResult<Vec<DeviceInfo>>> + MaybeSend;

    #[allow(clippy::type_complexity)]
    fn open(&self, id: &DeviceId, read: bool, write: bool) -> impl Future<Output = HidResult<(Option<Self::Reader>, Option<Self::Writer>)>> + MaybeSend;
    fn open_feature_handle(&self, id: &DeviceId) -> impl Future<Output = HidResult<Self::FeatureHandle>> + MaybeSend;

    async fn read_feature_report(&self, id: &DeviceId, buf: &mut [u8]) -> HidResult<usize> {
        let mut feature_buffer = self.open_feature_handle(id).await?;
        feature_buffer.read_feature_report(buf).await
    }
}

macro_rules! dyn_backend_impl {
    {
        $(
            $(#[$module_attrs:meta])*
            mod $module:ident {
                $(#[$item_attrs:meta])*
                $name:ident($backend:ty)
            }
        )+
    } => {
        $(
            $(#[$module_attrs])*
            mod $module;
        )+

        #[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
        #[non_exhaustive]
        pub enum BackendType {
            $(
                $(#[$module_attrs])*$(#[$item_attrs])*
                $name,
            )+
        }

        pub enum DynReader {
            $(
                $(#[$module_attrs])*$(#[$item_attrs])*
                $name(<$backend as Backend>::Reader),
            )+
        }
        impl AsyncHidRead for DynReader {
            async fn read_input_report<'a>(&'a mut self, buf: &'a mut [u8]) -> HidResult<usize> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.read_input_report(buf).await,
                    )+
                }
            }
        }

        pub enum DynWriter {
            $(
                $(#[$module_attrs])*$(#[$item_attrs])*
                $name(<$backend as Backend>::Writer),
            )+
        }
        impl AsyncHidWrite for DynWriter {
            async fn write_output_report<'a>(&'a mut self, buf: &'a [u8]) -> HidResult<()> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.write_output_report(buf).await,
                    )+
                }
            }
        }

        pub enum DynFeatureHandle {
            $(
                $(#[$module_attrs])*$(#[$item_attrs])*
                $name(<$backend as Backend>::FeatureHandle),
            )+
        }
        impl AsyncHidFeatureHandle for DynFeatureHandle {
            async fn read_feature_report<'a>(&'a mut self, buf: &'a mut [u8]) -> HidResult<usize> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.read_feature_report(buf).await,
                    )+
                }
            }

            async fn write_feature_report<'a>(&'a mut self, buf: &'a [u8]) -> HidResult<()> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.write_feature_report(buf).await,
                    )+
                }
            }
        }

         pub enum DynBackend {
            $(
                $(#[$module_attrs])*$(#[$item_attrs])*
                $name($backend),
            )+
        }
        impl DynBackend {
            pub fn new(backend: BackendType) -> DynBackend {
                match backend {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        BackendType::$name => Self::$name(<$backend as Default>::default()),
                    )+
                }
            }
        }
        impl Backend for DynBackend {
            type Reader = DynReader;
            type Writer = DynWriter;
            type FeatureHandle = DynFeatureHandle;

            async fn enumerate(&self) -> HidResult<DeviceInfoStream> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.enumerate().await,
                    )+
                }
            }

            fn watch(&self) -> HidResult<EventStream> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.watch(),
                    )+
                }
            }

             async fn query_info(&self, id: &DeviceId) -> HidResult<Vec<DeviceInfo>> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.query_info(id).await,
                    )+
                }
            }

            async fn open(&self, id: &DeviceId, read: bool, write: bool) -> HidResult<(Option<Self::Reader>, Option<Self::Writer>)> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.open(id, read, write).await.map(|(r, w)| (r.map(DynReader::$name), w.map(DynWriter::$name))),
                    )+
                }
            }

            async fn open_feature_handle(&self, id: &DeviceId) -> HidResult<Self::FeatureHandle> {
                match self {
                    $(
                        $(#[$module_attrs])*$(#[$item_attrs])*
                        Self::$name(i) => i.open_feature_handle(id).await.map(DynFeatureHandle::$name),
                    )+
                }
            }
        }
    };
}

// Rustfmt doesn't like my macro so we just declare them all with a bogus cfg attribute
#[cfg(rustfmt)]
mod hidraw;
#[cfg(rustfmt)]
mod iohidmanager;
#[cfg(rustfmt)]
mod webhid;
#[cfg(rustfmt)]
mod win32;
#[cfg(rustfmt)]
mod winrt;

// Dynamic dispatch doesn't play well with async traits so we just generate a big enum
// that forwards function calls the correct implementations
dyn_backend_impl! {
    #[cfg(all(target_os = "windows", feature = "win32"))]
    mod win32 {
        Win32(win32::Win32Backend)
    }
    #[cfg(all(target_os = "windows", feature = "winrt"))]
    mod winrt {
        WinRt(winrt::WinRtBackend)
    }
    #[cfg(target_os = "linux")]
    mod hidraw {
        HidRaw(hidraw::HidRawBackend)
    }
    #[cfg(target_os = "macos")]
    mod iohidmanager {
        IoHidManager(iohidmanager::IoHidManagerBackend)
    }
    #[cfg(target_arch = "wasm32")]
    mod webhid {
        WebHid(webhid::WebHidBackend)
    }
}

#[cfg(target_arch = "wasm32")]
pub use webhid::{request_device, RequestFilter};

impl Default for DynBackend {
    #[allow(unreachable_code)]
    fn default() -> Self {
        #[cfg(target_os = "windows")]
        {
            #[cfg(feature = "win32")]
            return Self::new(BackendType::Win32);
            #[cfg(feature = "winrt")]
            return Self::new(BackendType::WinRt);
        }
        #[cfg(target_os = "linux")]
        {
            return Self::new(BackendType::HidRaw);
        }
        #[cfg(target_os = "macos")]
        {
            return Self::new(BackendType::IoHidManager);
        }
        #[cfg(target_arch = "wasm32")]
        {
            return Self::new(BackendType::WebHid);
        }
        panic!("No suitable backend found");
    }
}
