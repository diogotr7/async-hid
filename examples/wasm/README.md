# async-hid WebHID Demo

A browser-based demo of async-hid using the WebHID API.

## Requirements

- [Trunk](https://trunkrs.dev/): `cargo install trunk`
- A browser that supports WebHID (Chrome, Edge, Opera — not Firefox)

## Running

```bash
trunk serve --open
```

This builds the wasm binary and serves it at `http://127.0.0.1:8080/`.

## Usage

1. Click **"Request Device"** to open the browser's HID device picker
2. Select a device — all its HID interfaces will be listed as buttons
3. Click an interface button to start reading input reports continuously
