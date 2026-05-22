# OwnWeb Agent Instructions

## Scope and Precedence

- Instructions apply by directory scope.
- Use the nearest `AGENTS.md` for files in that subtree.
- Rules are additive when they do not conflict.
- The nearest `AGENTS.md` rule wins for conflicts inside its subtree.
- For tasks that touch multiple scopes, satisfy all applicable rules and validation gates.

## Task Lifecycle

- Treat each direct execution request as one execution task.
- Complete this sequence for each execution task:
  1. Edit files for the requested scope.
  2. Run formatting for changed paths when a formatter exists.
  3. Run all required validation gates for changed paths.
  4. If the relevant git repo exists and gates pass, commit with `git commit --no-gpg-sign` unless the user says `do not commit`.
- Push only when explicitly requested.
- If any required gate fails, report failures and stop. Do not commit.
- If multiple git repos are changed, create separate commits per repo.
- Keep each commit scoped to one logical task and one git repo.
- Commit messages must answer why the change was made and summarize what it does.
- Keep all commit message lines at 72 columns or less.
- Do not sign commits.

## Required Validation Gates

- For daemon changes, follow `daemon/AGENTS.md`.
- For Chrome extension changes, follow `google-chrome-extension/AGENTS.md`.
- When a root-level workflow file exists, run the relevant root check target before commit.
- If checks are intentionally limited, state exactly what was run and why.

## Runtime Boundary Policy

- Treat the browser extension and daemon boundary as a hard architecture boundary.
- The Chrome extension is a generic client-side DOM bridge:
  - observes page DOM changes
  - captures bounded candidate DOM region snapshots
  - sends page metadata and snapshots to the local daemon
  - applies daemon DOM commands to the browser view
- The Rust daemon is authoritative for filtering decisions:
  - site-specific DOM interpretation
  - normalized content storage
  - spam or generated-content classification
  - final `keep`, `hide`, `dim`, `insertLabel`, or `replaceText` commands
  - command reasons and confidence values
- Do not place authoritative filtering policy in the extension.
- Keep the REST contract explicit and documented.

## Documentation Policy

- Keep docs updated when behavior, commands, or JSON contracts change.
- Add Rust doc comments for new public structs, enums, fields, methods, and free functions where documentation improves clarity.
- Document invariants and validation expectations where they affect runtime behavior.
- Do not leak information from sibling or private repos into this project.

## Generated Local State

- Build output and local capture files are generated state.
- Do not commit `target/`, `captured-posts.jsonl`, or browser-generated extension state.
- Remove generated build output before final status checks when it is not needed.

## Writing Style

- Write tight, technical, declarative prose.
- Prefer concrete nouns and verbs.
- Every sentence should add new information.
- Avoid low-signal contrastive phrasing that restates what the user is not doing or does not want unless that distinction prevents a concrete misunderstanding.
