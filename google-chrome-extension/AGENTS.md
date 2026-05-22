# Chrome Extension Agent Instructions

## Scope

- These rules apply to tasks that change files under `google-chrome-extension/`.
- These rules are additive with the repository root `AGENTS.md`.

## Validation Gates

- Run syntax checks for changed JavaScript files before commit:
  - `node --check background.js`
  - `node --check contentScript.js`
  - `node --check options.js`
- If a package manager or bundler is added later, run the relevant lint, test, and build commands before commit.
- For daemon contract changes, verify the extension and daemon request/response shapes still match.

## Extension Policy

- Keep daemon communication in the background service worker.
- Keep content scripts focused on DOM observation, extraction, and applying returned decisions.
- Avoid placing authoritative filtering policy in the extension.
- Preserve graceful behavior when the daemon is unavailable.
- Avoid repeatedly processing DOM mutations caused by the extension itself.

## Browser Testing

- Manual Chrome testing requires reloading the unpacked extension at `chrome://extensions`.
- Restart the daemon when daemon code changes.
- Refresh the target page after extension or daemon changes that affect content-script behavior.
