# OwnWeb

Chrome Manifest V3 extension that sends generic DOM region snapshots to a
local OwnWeb daemon and applies the daemon's returned DOM commands.

## Load in Chrome

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose Load unpacked.
4. Select this `google-chrome-extension` directory.

## Daemon contract

The extension sends batched requests to:

```http
POST http://127.0.0.1:17891/v1/dom/analyze
Content-Type: application/json
```

Request shape:

```json
{
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

Response shape:

```json
{
  "commands": [
    {
      "action": "insertLabel",
      "target": {
        "clientId": "dom:1",
        "selector": "article:nth-of-type(1)",
        "mustMatchSnapshotHash": "abc123"
      },
      "label": "Summary: Post summary",
      "text": null,
      "reason": "Codex app-server summary",
      "confidence": 0.8
    }
  ]
}
```

Supported actions are `keep`, `hide`, `dim`, `insertLabel`, and
`replaceText`. The extension does not make site-specific filtering decisions;
the daemon interprets supported sites and decides what commands to return.
