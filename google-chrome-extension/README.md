# OwnWeb

Chrome Manifest V3 extension that sends generic DOM region snapshots to a
local OwnWeb daemon and applies the daemon's returned DOM commands.

## Load in Chrome

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose Load unpacked.
4. Select this `google-chrome-extension` directory.

## Daemon contract

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

The extension also sends user feedback to the daemon. A thumbs-down click posts
the current region snapshot to:

```http
POST http://127.0.0.1:17891/v1/dom/feedback
Content-Type: application/json
```

For X posts, the daemon stores feedback events and current feedback state in the
X site database. The first thumbs-down records the dislike and opens a local
reason panel; later scans hide posts whose stored feedback state is active.

WebSocket request shape:

```json
{
  "type": "analyzeDom",
  "requestId": "dom:1",
  "page": {
    "url": "https://x.com/home",
    "title": "X",
    "capturedAt": "2026-05-22T10:00:00.000Z"
  },
  "elements": [
    {
      "clientId": "dom:1",
      "selector": "article:nth-of-type(1)",
      "tagName": "article",
      "role": "article",
      "text": "Post text",
      "html": "<article>...</article>",
      "attributes": [],
      "links": [
        {
          "href": "https://x.com/user/status/123",
          "text": "status",
          "ariaLabel": null
        }
      ],
      "snapshotHash": "abc123",
      "capturedAt": "2026-05-21T10:00:00.000Z"
    }
  ]
}
```

WebSocket command event shape:

```json
{
  "type": "commands",
  "requestId": "dom:1",
  "phase": "final",
  "commands": [
    {
      "action": "insertFeedbackControl",
      "target": {
        "clientId": "dom:1",
        "selector": "article:nth-of-type(1)",
        "mustMatchSnapshotHash": "abc123"
      },
      "label": "Hide this post",
      "text": null,
      "reason": "User feedback control",
      "confidence": null
    }
  ]
}
```

Supported actions are `keep`, `hide`, `dim`, `insertLabel`,
`insertFeedbackControl`, and `replaceText`. The extension does not make
site-specific filtering decisions; the daemon interprets supported sites and
decides what commands to return.

Feedback request shape:

```json
{
  "feedback": "thumbsDown",
  "reason": "",
  "page": {
    "url": "https://x.com/home",
    "title": "X",
    "capturedAt": "2026-05-22T10:00:00.000Z"
  },
  "element": {
    "clientId": "dom:1",
    "selector": "article:nth-of-type(1)",
    "tagName": "article",
    "role": "article",
    "text": "Post text",
    "html": "<article>...</article>",
    "attributes": [],
    "links": [
      {
        "href": "https://x.com/user/status/123",
        "text": "status",
        "ariaLabel": null
      }
    ],
    "snapshotHash": "abc123",
    "capturedAt": "2026-05-21T10:00:00.000Z"
  }
}
```

Supported feedback values are `thumbsDown`, `undoThumbsDown`, and
`updateReason`.
