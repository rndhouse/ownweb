# Installation

WebLayer's daemon and CLI are published on crates.io as the `weblayer` crate.

## Prerequisites

- Rust and Cargo
- Chrome, Firefox, or another browser that can load development extensions
- Git, for loading the extension from the source repository

## Install the Binary

```sh
cargo install weblayer
```

Start the local daemon:

```sh
weblayer daemon
```

The daemon listens on `127.0.0.1:17891` by default.

In another terminal, check that the CLI can reach it:

```sh
weblayer status
```

## Load the Browser Extension

The browser extension is currently loaded from the source repository.

```sh
git clone https://github.com/rndhouse/weblayer.git
cd weblayer
make -C browser-extension
```

For Chrome:

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose **Load unpacked**.
4. Select `browser-extension/dist/chrome` from the repository checkout.

For Firefox:

1. Open `about:debugging`.
2. Choose **This Firefox**.
3. Choose **Load Temporary Add-on**.
4. Select `browser-extension/dist/firefox/manifest.json`.

After the extension is loaded, visit a supported site while the daemon is
running.

## Update the Binary

```sh
cargo install --force weblayer
```
