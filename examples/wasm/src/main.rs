use async_hid::{request_device, DeviceInfo, HidBackend};
use futures_lite::StreamExt;
use wasm_bindgen::prelude::*;
use web_sys::window;

fn log_event(msg: &str) {
    let document = window().unwrap().document().unwrap();
    let el = document.get_element_by_id("events").unwrap();
    let current = el.text_content().unwrap_or_default();
    el.set_text_content(Some(&format!("{current}{msg}\n")));
    el.set_scroll_top(el.scroll_height() as f64);
}

fn log_report(msg: &str) {
    let document = window().unwrap().document().unwrap();
    let el = document.get_element_by_id("reports").unwrap();
    el.set_text_content(Some(msg));
}

fn set_status(msg: &str) {
    let document = window().unwrap().document().unwrap();
    let el = document.get_element_by_id("status").unwrap();
    el.set_text_content(Some(msg));
}

fn add_device_button(device: &DeviceInfo) {
    let document = window().unwrap().document().unwrap();
    let container = document.get_element_by_id("devices").unwrap();

    let btn = document.create_element("button").unwrap();
    let label = format!(
        "Read: {} (0x{:04X}:0x{:04X})",
        device.name, device.usage_page, device.usage_id
    );
    btn.set_text_content(Some(&label));

    let device_id = device.id.clone();
    let device_name = device.name.clone();
    let usage_page = device.usage_page;
    let usage_id = device.usage_id;
    let cb = Closure::wrap(Box::new(move || {
        let device_id = device_id.clone();
        let device_name = device_name.clone();
        wasm_bindgen_futures::spawn_local(async move {
            log_event(&format!("Opening {} (0x{:04X}:0x{:04X})...", device_name, usage_page, usage_id));
            let backend = HidBackend::default();
            let devices = match backend.query_devices(&device_id).await {
                Ok(d) => d,
                Err(e) => {
                    log_event(&format!("Query error: {e}"));
                    return;
                }
            };
            let device = match devices.into_iter().next() {
                Some(d) => d,
                None => {
                    log_event("Device not found.");
                    return;
                }
            };
            match device.open_readable().await {
                Ok(mut reader) => {
                    use async_hid::AsyncHidRead;
                    log_event(&format!("Listening on {} (0x{:04X}:0x{:04X})", device_name, usage_page, usage_id));
                    set_status(&format!("Listening on {}...", device_name));
                    let mut buf = [0u8; 64];
                    loop {
                        match reader.read_input_report(&mut buf).await {
                            Ok(n) => {
                                log_report(&format!("{:02X?}", &buf[..n]));
                            }
                            Err(e) => {
                                log_event(&format!("Disconnected: {e}"));
                                set_status("Disconnected.");
                                break;
                            }
                        }
                    }
                }
                Err(e) => log_event(&format!("Open error: {e}")),
            }
        });
    }) as Box<dyn FnMut()>);

    btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    container.append_child(&btn).unwrap();
}

fn main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).ok();

    let document = window().unwrap().document().unwrap();

    let request_btn = document.get_element_by_id("request-btn").unwrap();
    let request_cb = Closure::wrap(Box::new(move || {
        wasm_bindgen_futures::spawn_local(async {
            set_status("Requesting device...");
            log_event("Requesting device...");
            match request_device(&[]).await {
                Ok(devices) => {
                    if devices.is_empty() {
                        set_status("No device selected.");
                        log_event("No device selected.");
                        return;
                    }
                    log_event("Device granted. Enumerating interfaces...");

                    let backend = HidBackend::default();
                    let mut all = match backend.enumerate().await {
                        Ok(d) => d,
                        Err(e) => {
                            set_status(&format!("Enumerate error: {e}"));
                            return;
                        }
                    };

                    let mut count = 0;
                    while let Some(device) = all.next().await {
                        log_event(&format!(
                            "Found: {} (VID: 0x{:04X}, PID: 0x{:04X}, Usage: 0x{:04X}:0x{:04X})",
                            device.name, device.vendor_id, device.product_id, device.usage_page, device.usage_id
                        ));
                        add_device_button(&device);
                        count += 1;
                    }
                    set_status(&format!("{count} interface(s) found. Click a button to read."));
                }
                Err(e) => {
                    set_status(&format!("Error: {e}"));
                    log_event(&format!("Error: {e}"));
                }
            }
        });
    }) as Box<dyn FnMut()>);
    request_btn
        .add_event_listener_with_callback("click", request_cb.as_ref().unchecked_ref())
        .unwrap();
    request_cb.forget();
}
