# Chrome Extension

The Chrome extension sends generic DOM region snapshots to the local WebLayer
daemon and applies the daemon's returned DOM commands.

## Load in Chrome

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose **Load unpacked**.
4. Select the `google-chrome-extension` directory from the repository checkout.

## Daemon Contract

The extension opens a WebSocket to:

```http
GET ws://127.0.0.1:17891/v1/events
```

It sends DOM analysis events and receives command events. The daemon can push
immediate `pending` commands, such as `insertFeedbackControl`, before local
analysis finishes. When a final answer is already cached, the daemon can push
`final` immediately without a `pending` event.

The REST endpoint remains available as a fallback and smoke-test path:

```http
POST http://127.0.0.1:17891/v1/dom/analyze
Content-Type: application/json
```

The extension also sends user feedback to the daemon:

```http
POST http://127.0.0.1:17891/v1/dom/feedback
Content-Type: application/json
```

Supported DOM command actions are `keep`, `hide`, `dim`, `insertLabel`,
`insertFeedbackControl`, and `replaceText`.

The extension does not make site-specific filtering decisions. The daemon
interprets supported sites and decides what commands to return.
