# WebLayer Rules

Content rules are stored as site-scoped policy records in the site database.
For X, rules live in `~/.local/share/weblayer/x.com/db.sqlite` in the
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
GET http://127.0.0.1:17891/v1/rules?site=x.com
```

Supported query parameters:

- `site`: required site scope, such as `x.com`.
- `status`: optional status filter, such as `active`.
- `limit`: maximum rows to return; defaults to `100` and is capped at `500`.
- `offset`: rows to skip.

## Lifecycle

Supported statuses are:

- `draft`: editable rule that is not used for filtering.
- `active`: rule sent to the AI analyzer for X post filtering.
- `disabled`: retained rule that is not used for filtering.
- `archived`: retained historical rule hidden from normal active management.

New rules created through the daemon default to `draft`. Activating a rule is an
explicit status change.

## Management API

Create a draft rule:

```http
POST http://127.0.0.1:17891/v1/rules?site=x.com
Content-Type: application/json
```

```json
{
  "id": "x-ai-slop",
  "title": "AI slop",
  "instruction": "Hide generic AI engagement bait.",
  "source": "user",
  "examples": {
    "positive": ["I asked ChatGPT to write this viral thread"],
    "negative": ["Detailed notes about local AI implementation"]
  }
}
```

Show a rule and its recent audit events:

```http
GET http://127.0.0.1:17891/v1/rules/x-ai-slop?site=x.com
```

Update any subset of title, instruction, status, priority, source, and examples:

```http
POST http://127.0.0.1:17891/v1/rules/x-ai-slop?site=x.com
Content-Type: application/json
```

```json
{
  "priority": 25,
  "examples": {
    "positive": ["reply with yes if you agree"]
  },
  "source": "user"
}
```

Change status:

```http
POST http://127.0.0.1:17891/v1/rules/x-ai-slop/status?site=x.com
Content-Type: application/json
```

```json
{
  "status": "active",
  "source": "user"
}
```

Test a rule against stored X posts before activating it:

```http
GET http://127.0.0.1:17891/v1/rules/x-ai-slop/validate?site=x.com&limit=20
```

Validation is a local heuristic over stored post text, rule terms, and examples.
It is intended to catch broad or vague rules before activation.

Suggest draft candidates from active feedback reasons:

```http
GET http://127.0.0.1:17891/v1/rule-suggestions?site=x.com&minFeedback=2
```

Suggestions are review material only. They are not inserted into
`content_rules`, and they are not active. Create a draft rule from a suggestion
only after reviewing its instruction and evidence examples.

## CLI

```sh
cargo run -- rules list --site x.com
cargo run -- rules show x-ai-slop --site x.com
cargo run -- rules create --site x.com --id x-ai-slop \
  --title "AI slop" \
  --instruction "Hide generic AI engagement bait." \
  --positive-example "I asked ChatGPT to write this viral thread"
cargo run -- rules suggest --site x.com --min-feedback 2
cargo run -- rules validate x-ai-slop --site x.com
cargo run -- rules enable x-ai-slop --site x.com
cargo run -- rules disable x-ai-slop --site x.com
cargo run -- rules archive x-ai-slop --site x.com
```
