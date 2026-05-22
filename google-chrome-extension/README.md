# OwnWeb

Chrome Manifest V3 extension that sends website content to a local OwnWeb daemon and applies the daemon's returned action. The current implementation supports X/Twitter posts.

## Load in Chrome

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose Load unpacked.
4. Select this `google-chrome-extension` directory.

## Daemon contract

The extension sends batched requests to:

```http
POST http://127.0.0.1:17891/v1/x-posts/analyze
Content-Type: application/json
```

Request shape:

```json
{
  "source": "x.com",
  "posts": [
    {
      "clientId": "x:123:1",
      "postId": "123",
      "url": "https://x.com/user/status/123",
      "authorHandle": "@user",
      "text": "Post text",
      "capturedAt": "2026-05-21T10:00:00.000Z"
    }
  ]
}
```

Response shape:

```json
{
  "posts": [
    {
      "clientId": "x:123:1",
      "action": "hide",
      "label": "OwnWeb: spam",
      "reason": "Promotional spam",
      "replacementText": null,
      "confidence": 0.91
    }
  ]
}
```

Supported actions are `keep`, `hide`, `dim`, `label`, and `replace`.
