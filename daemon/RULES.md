# OwnWeb Rules

Content rules are stored as site-scoped policy records in the site database.
For X, rules live in `~/.local/share/ownweb/x.com/db.sqlite` in the
`content_rules` table.

The daemon currently seeds one active X rule:

```json
{
  "id": "x-engagement-bait-reaction",
  "site": "x.com",
  "status": "active",
  "priority": 50,
  "title": "Engagement bait reaction posts",
  "instruction": "Downrank engagement bait, dunking, or 'look at this absurd thing' posts where the main content is a reaction to a video, image, or quote rather than a substantive claim.",
  "createdSource": "user",
  "examples": {
    "positive": [],
    "negative": []
  }
}
```

Rules can be inspected through:

```http
GET http://127.0.0.1:17891/v1/sites/x.com/rules
```

Supported query parameters:

- `status`: optional status filter, such as `active`.
- `limit`: maximum rows to return; defaults to `100` and is capped at `500`.
- `offset`: rows to skip.

