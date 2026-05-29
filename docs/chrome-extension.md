# Chrome Extension

The Chrome extension sends allowlisted site content snapshots to the local
WebLayer daemon and applies the daemon's returned DOM commands.

## Load in Chrome

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose **Load unpacked**.
4. Select the `google-chrome-extension` directory from the repository checkout.

## Daemon Contract

The extension captures content only through explicit site adapters. Unknown
hosts, unknown page surfaces, and unknown DOM structures send nothing. The
current adapter supports X.com and Twitter URLs for home, explore, search, and
status-thread pages, and it captures visible top-level
`article[data-testid="tweet"]` post regions. Quoted posts and reply/thread
context stay inside the captured post snapshot; nested quoted-post cards are
not targeted as separate feedback regions.

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

The extension decides only which site content surfaces may be captured. It does
not make filtering decisions. The daemon interprets captured content and
decides what commands to return.
