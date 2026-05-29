# Installation

WebLayer currently runs from a source checkout.

## Prerequisites

- Rust and Cargo
- Chrome or a Chromium-based browser that can load unpacked extensions
- Git

## Get the Source

```sh
git clone https://github.com/rndhouse/weblayer.git
cd weblayer
```

## Start the Daemon

```sh
cargo run --manifest-path weblayer/Cargo.toml -- daemon
```

The daemon listens on `127.0.0.1:17891` by default.

## Check the CLI

In another terminal, run:

```sh
cargo run --manifest-path weblayer/Cargo.toml -- status
```

## Load the Extension

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose **Load unpacked**.
4. Select the `google-chrome-extension` directory from the repository checkout.

After the extension is loaded, visit a supported site while the daemon is
running.
