# Installation

WebLayer's daemon and CLI are published on crates.io as the `weblayer` crate.

## Prerequisites

- Rust and Cargo
- Chrome or a Chromium-based browser that can load unpacked extensions
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

## Load the Extension

The Chrome extension is currently loaded from the source repository.

```sh
git clone https://github.com/rndhouse/weblayer.git
cd weblayer
```

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose **Load unpacked**.
4. Select the `google-chrome-extension` directory from the repository checkout.

After the extension is loaded, visit a supported site while the daemon is
running.

## Update the Binary

```sh
cargo install --force weblayer
```
