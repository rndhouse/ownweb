<h1 align="center">WebLayer</h1>

<p align="center"><strong>Control the web you consume.</strong></p>

WebLayer helps you take sovereignty over the web you consume. It sends page content from your browser to a local daemon, where your own rules and AI agents can learn from your feedback, hide what you do not want to see, and keep relevant browsing data under your control.

<p align="center">
  <img src="assets/architecture.svg" alt="Browser to WebLayer extension to WebLayer daemon, with an AI agent and content store connected to the daemon" width="720">
</p>

## Usage

WebLayer builds as one `weblayer` binary. Start the local daemon with:

```sh
cargo run --manifest-path daemon/Cargo.toml -- daemon
```

After the daemon is running, use the same binary as a CLI client:

```sh
cargo run --manifest-path daemon/Cargo.toml -- status
cargo run --manifest-path daemon/Cargo.toml -- rules list --site x.com
cargo run --manifest-path daemon/Cargo.toml -- content stats --site x.com
```

## Supported Sites

### X.com

WebLayer currently supports X.com posts, adding a local dislike control so you can hide posts and teach the daemon what you do not want to see.

<p align="center">
  <img src="assets/x-dislike-button.png" alt="WebLayer dislike button on an X.com post" width="520">
</p>
