use std::cell::RefCell;
use std::rc::Rc;

use futures_channel::mpsc;
use futures_lite::stream;
use futures_lite::StreamExt;
use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HidCollectionInfo, HidConnectionEvent, HidDevice, HidDeviceFilter, HidDeviceRequestOptions, HidInputReportEvent};

use crate::backend::{Backend, DeviceInfoStream, EventStream};
use crate::traits::{AsyncHidFeatureHandle, AsyncHidRead, AsyncHidWrite};

use crate::device_info::DeviceId;
use crate::error::HidResult;
use crate::{DeviceEvent, DeviceInfo, HidError};

// Thread-local storage for granted HidDevice JS objects.
// We store devices here so that DeviceId::WebHid(index) remains stable
// between request_device(), enumerate(), and open() calls.
thread_local! {
    static DEVICES: RefCell<Vec<HidDevice>> = RefCell::new(Vec::new());
}

fn get_hid() -> HidResult<web_sys::Hid> {
    let window = web_sys::window().ok_or(HidError::message("No window object available"))?;
    let hid = window.navigator().hid();
    if hid.is_undefined() {
        return Err(HidError::message("WebHID is not supported in this browser"));
    }
    Ok(hid)
}

fn get_stored_device(index: u32) -> HidResult<HidDevice> {
    DEVICES.with(|devices| {
        devices
            .borrow()
            .get(index as usize)
            .cloned()
            .ok_or(HidError::NotConnected)
    })
}

fn store_device(device: HidDevice) -> u32 {
    DEVICES.with(|devices| {
        let mut devices = devices.borrow_mut();
        // Check if this device is already stored (by reference equality)
        for (i, existing) in devices.iter().enumerate() {
            if existing == &device {
                return i as u32;
            }
        }
        let index = devices.len() as u32;
        devices.push(device);
        index
    })
}

/// Sync the thread-local device store with the browser's getDevices() list.
/// Returns DeviceInfo for all currently granted devices.
pub(crate) async fn sync_devices() -> HidResult<Vec<DeviceInfo>> {
    let hid = get_hid()?;
    let js_devices = JsFuture::from(hid.get_devices().unchecked_into::<js_sys::Promise>())
        .await
        .map_err(HidError::from)?;
    let js_devices: js_sys::Array = js_devices.unchecked_into();

    let mut infos = Vec::new();
    for i in 0..js_devices.length() {
        let device: HidDevice = js_devices.get(i).unchecked_into();
        let index = store_device(device.clone());
        infos.extend(device_infos_from_hid_device(&device, index));
    }
    Ok(infos)
}

/// Compute the max report byte length from a JS array of HidReportInfo.
/// Each report has `items` (array of HidReportItem with reportSize/reportCount in bits).
/// Returns the max bytes across all reports, plus 1 for the report ID prefix byte.
fn max_report_byte_length(reports: &js_sys::Array) -> Option<u16> {
    if reports.length() == 0 {
        return None;
    }

    let mut max_bytes: u32 = 0;
    for i in 0..reports.length() {
        let report: JsValue = reports.get(i);
        let items = js_sys::Reflect::get(&report, &"items".into())
            .ok()
            .and_then(|v| v.dyn_into::<js_sys::Array>().ok());

        let Some(items) = items else { continue };

        let mut total_bits: u32 = 0;
        for j in 0..items.length() {
            let item = items.get(j);
            let size = js_sys::Reflect::get(&item, &"reportSize".into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as u32;
            let count = js_sys::Reflect::get(&item, &"reportCount".into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as u32;
            total_bits += size * count;
        }
        let bytes = (total_bits + 7) / 8;
        max_bytes = max_bytes.max(bytes);
    }

    // Add 1 for the report ID prefix byte (matching native HIDP_CAPS behavior)
    Some((max_bytes + 1) as u16)
}

/// Returns one DeviceInfo per collection on the device, matching native behavior
/// where each HID interface appears as a separate device in enumeration.
fn device_infos_from_hid_device(device: &HidDevice, index: u32) -> Vec<DeviceInfo> {
    let collections = device.collections();
    let base = DeviceInfo {
        id: DeviceId::WebHid(index),
        name: device.product_name(),
        manufacturer: None,
        product_id: device.product_id(),
        vendor_id: device.vendor_id(),
        usage_id: 0,
        usage_page: 0,
        serial_number: None,
        max_input_report_size: None,
        max_output_report_size: None,
        max_feature_report_size: None,
    };

    if collections.length() == 0 {
        return vec![base];
    }

    (0..collections.length())
        .map(|i| {
            let collection: HidCollectionInfo = collections.get(i).unchecked_into();
            DeviceInfo {
                usage_page: collection.get_usage_page().unwrap_or(0),
                usage_id: collection.get_usage().unwrap_or(0),
                max_input_report_size: js_sys::Reflect::get(&collection, &"inputReports".into())
                    .ok().and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                    .and_then(|a| max_report_byte_length(&a)),
                max_output_report_size: js_sys::Reflect::get(&collection, &"outputReports".into())
                    .ok().and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                    .and_then(|a| max_report_byte_length(&a)),
                max_feature_report_size: js_sys::Reflect::get(&collection, &"featureReports".into())
                    .ok().and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                    .and_then(|a| max_report_byte_length(&a)),
                ..base.clone()
            }
        })
        .collect()
}

/// Filter for `request_device()`. All `None` fields are omitted (match anything).
#[derive(Debug, Default, Clone)]
pub struct RequestFilter {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
}

fn build_filters(filters: &[RequestFilter]) -> Vec<HidDeviceFilter> {
    filters
        .iter()
        .filter(|f| f.vendor_id.is_some() || f.product_id.is_some() || f.usage_page.is_some() || f.usage.is_some())
        .map(|f| {
            let filter = HidDeviceFilter::new();
            if let Some(v) = f.vendor_id {
                filter.set_vendor_id(v as u32);
            }
            if let Some(v) = f.product_id {
                filter.set_product_id(v);
            }
            if let Some(v) = f.usage_page {
                filter.set_usage_page(v);
            }
            if let Some(v) = f.usage {
                filter.set_usage(v);
            }
            filter
        })
        .collect()
}

/// Request access to HID devices via the browser's device picker.
///
/// This **must** be called from a user gesture (click handler, etc.) due to browser security.
/// Returns the list of devices the user selected. The devices are stored internally
/// and will appear in subsequent `enumerate()` calls.
///
/// `filters` constrains which devices appear in the picker. An empty slice shows all HID devices.
pub async fn request_device(filters: &[RequestFilter]) -> HidResult<Vec<DeviceInfo>> {
    let hid = get_hid()?;

    let built_filters = build_filters(filters);
    // Construct HidDeviceRequestOptions manually to avoid web-sys version differences
    // in the signature of HidDeviceRequestOptions::new()
    let filter_array: js_sys::Array = built_filters.into_iter().map(JsValue::from).collect();
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"filters".into(), &filter_array);
    let options: HidDeviceRequestOptions = options.unchecked_into();
    let devices = JsFuture::from(hid.request_device(&options).unchecked_into::<js_sys::Promise>())
        .await
        .map_err(HidError::from)?;
    let devices: js_sys::Array = devices.unchecked_into();

    let mut result = Vec::new();
    for i in 0..devices.length() {
        let device: HidDevice = devices.get(i).unchecked_into();
        let index = store_device(device.clone());
        result.extend(device_infos_from_hid_device(&device, index));
    }
    Ok(result)
}

// --- Backend ---

#[derive(Default)]
pub struct WebHidBackend;

impl Backend for WebHidBackend {
    type Reader = WebHidReader;
    type Writer = WebHidWriter;
    type FeatureHandle = WebHidFeatureHandle;

    async fn enumerate(&self) -> HidResult<DeviceInfoStream> {
        let infos = sync_devices().await?;
        let stream = stream::iter(infos.into_iter().map(Ok));
        Ok(Box::pin(stream))
    }

    fn watch(&self) -> HidResult<EventStream> {
        let hid = get_hid()?;
        let (sender, receiver) = mpsc::unbounded();

        let connect_sender = sender.clone();
        let on_connect = Closure::wrap(Box::new(move |event: HidConnectionEvent| {
            let device = event.device();
            let index = store_device(device);
            let _ = connect_sender.unbounded_send(DeviceEvent::Connected(DeviceId::WebHid(index)));
        }) as Box<dyn FnMut(HidConnectionEvent)>);
        hid.set_onconnect(Some(on_connect.as_ref().unchecked_ref()));
        on_connect.forget();

        let on_disconnect = Closure::wrap(Box::new(move |event: HidConnectionEvent| {
            let device = event.device();
            // Look up the device in our store. If not found, we can't emit a meaningful event.
            let index = DEVICES.with(|devices| {
                devices.borrow().iter().enumerate()
                    .find(|(_, existing)| *existing == &device)
                    .map(|(i, _)| i as u32)
            });
            if let Some(index) = index {
                let _ = sender.unbounded_send(DeviceEvent::Disconnected(DeviceId::WebHid(index)));
            }
        }) as Box<dyn FnMut(HidConnectionEvent)>);
        hid.set_ondisconnect(Some(on_disconnect.as_ref().unchecked_ref()));
        on_disconnect.forget();

        Ok(Box::pin(receiver))
    }

    async fn query_info(&self, id: &DeviceId) -> HidResult<Vec<DeviceInfo>> {
        let DeviceId::WebHid(index) = id;
        let device = get_stored_device(*index)?;
        Ok(device_infos_from_hid_device(&device, *index))
    }

    async fn open(&self, id: &DeviceId, read: bool, write: bool) -> HidResult<(Option<Self::Reader>, Option<Self::Writer>)> {
        let DeviceId::WebHid(index) = id;
        let device = get_stored_device(*index)?;

        if !device.opened() {
            JsFuture::from(device.open().unchecked_into::<js_sys::Promise>())
                .await
                .map_err(HidError::from)?;
        }

        let device = Rc::new(device);

        let reader = if read {
            Some(WebHidReader::new(device.clone()))
        } else {
            None
        };
        let writer = if write {
            Some(WebHidWriter { device: device.clone() })
        } else {
            None
        };

        Ok((reader, writer))
    }

    async fn open_feature_handle(&self, id: &DeviceId) -> HidResult<Self::FeatureHandle> {
        let DeviceId::WebHid(index) = id;
        let device = get_stored_device(*index)?;

        if !device.opened() {
            JsFuture::from(device.open().unchecked_into::<js_sys::Promise>())
                .await
                .map_err(HidError::from)?;
        }

        Ok(WebHidFeatureHandle { device: Rc::new(device) })
    }
}

// --- Reader ---

pub struct WebHidReader {
    _device: Rc<HidDevice>,
    receiver: mpsc::UnboundedReceiver<Vec<u8>>,
    _closure: Closure<dyn FnMut(HidInputReportEvent)>,
}

impl WebHidReader {
    fn new(device: Rc<HidDevice>) -> Self {
        let (sender, receiver) = mpsc::unbounded();

        let closure = Closure::wrap(Box::new(move |event: HidInputReportEvent| {
            let data_view = event.data();
            let byte_length = data_view.byte_length();
            let mut buf = vec![0u8; byte_length + 1];
            buf[0] = event.report_id();
            let uint8 = Uint8Array::new_with_byte_offset_and_length(
                &data_view.buffer(),
                data_view.byte_offset() as u32,
                byte_length as u32,
            );
            uint8.copy_to(&mut buf[1..]);
            let _ = sender.unbounded_send(buf);
        }) as Box<dyn FnMut(HidInputReportEvent)>);

        device.set_oninputreport(Some(closure.as_ref().unchecked_ref()));

        Self {
            _device: device,
            receiver,
            _closure: closure,
        }
    }
}

impl AsyncHidRead for WebHidReader {
    async fn read_input_report<'a>(&'a mut self, buf: &'a mut [u8]) -> HidResult<usize> {
        let data = self.receiver.next().await
            .ok_or(HidError::Disconnected)?;
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        Ok(len)
    }
}

// --- Writer ---

pub struct WebHidWriter {
    device: Rc<HidDevice>,
}

impl AsyncHidWrite for WebHidWriter {
    async fn write_output_report<'a>(&'a mut self, buf: &'a [u8]) -> HidResult<()> {
        if buf.is_empty() {
            return Err(HidError::message("Output report must contain at least the report ID byte"));
        }
        let report_id = buf[0];
        let data = Uint8Array::from(&buf[1..]);
        let promise = self.device.send_report_with_u8_array(report_id, &data)
            .map_err(HidError::from)?;
        JsFuture::from(promise.unchecked_into::<js_sys::Promise>())
            .await
            .map_err(HidError::from)?;
        Ok(())
    }
}

// --- Feature Handle ---

pub struct WebHidFeatureHandle {
    device: Rc<HidDevice>,
}

impl AsyncHidFeatureHandle for WebHidFeatureHandle {
    async fn read_feature_report<'a>(&'a mut self, buf: &'a mut [u8]) -> HidResult<usize> {
        if buf.is_empty() {
            return Err(HidError::message("Buffer must contain at least the report ID byte"));
        }
        let report_id = buf[0];
        let result = JsFuture::from(
            self.device.receive_feature_report(report_id).unchecked_into::<js_sys::Promise>()
        )
            .await
            .map_err(HidError::from)?;
        let data_view: js_sys::DataView = result.unchecked_into();
        let byte_length = data_view.byte_length();
        let len = byte_length.min(buf.len() - 1);
        let uint8 = Uint8Array::new_with_byte_offset_and_length(
            &data_view.buffer(),
            data_view.byte_offset() as u32,
            len as u32,
        );
        uint8.copy_to(&mut buf[1..len + 1]);
        Ok(len + 1)
    }

    async fn write_feature_report<'a>(&'a mut self, buf: &'a [u8]) -> HidResult<()> {
        if buf.is_empty() {
            return Err(HidError::message("Feature report must contain at least the report ID byte"));
        }
        let report_id = buf[0];
        let data = Uint8Array::from(&buf[1..]);
        let promise = self.device.send_feature_report_with_u8_array(report_id, &data)
            .map_err(HidError::from)?;
        JsFuture::from(promise.unchecked_into::<js_sys::Promise>())
            .await
            .map_err(HidError::from)?;
        Ok(())
    }
}
