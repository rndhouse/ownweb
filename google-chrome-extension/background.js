const DEFAULT_DAEMON_ORIGIN = "http://127.0.0.1:17891";
const ANALYZE_PATH = "/v1/x-posts/analyze";
const REQUEST_TIMEOUT_MS = 20000;
const MAX_POSTS_PER_REQUEST = 8;

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || message.type !== "pairpilot:analyzePosts") {
    return false;
  }

  analyzePosts(message.posts)
    .then((decisions) => sendResponse({ ok: true, decisions }))
    .catch((error) => {
      sendResponse({
        ok: false,
        error: error instanceof Error ? error.message : String(error)
      });
    });

  return true;
});

async function analyzePosts(posts) {
  if (!Array.isArray(posts)) {
    throw new Error("Expected posts to be an array.");
  }

  const safePosts = posts.slice(0, MAX_POSTS_PER_REQUEST).map(normalizePost);
  if (safePosts.length === 0) {
    return [];
  }

  const settings = await getSettings();
  const response = await postJson(settings.daemonOrigin, ANALYZE_PATH, {
    source: "x.com",
    posts: safePosts
  });

  if (!Array.isArray(response.posts)) {
    throw new Error("Daemon response must include a posts array.");
  }

  return response.posts.map(normalizeDecision);
}

function normalizePost(post) {
  return {
    clientId: stringOrEmpty(post.clientId),
    postId: stringOrNull(post.postId),
    url: stringOrNull(post.url),
    authorHandle: stringOrNull(post.authorHandle),
    text: stringOrEmpty(post.text),
    capturedAt: stringOrNull(post.capturedAt)
  };
}

function normalizeDecision(decision) {
  const action = stringOrEmpty(decision.action).toLowerCase();
  const allowedActions = new Set(["keep", "hide", "dim", "label", "replace"]);

  return {
    clientId: stringOrEmpty(decision.clientId),
    action: allowedActions.has(action) ? action : "keep",
    label: stringOrNull(decision.label),
    reason: stringOrNull(decision.reason),
    replacementText: stringOrNull(decision.replacementText),
    confidence: Number.isFinite(decision.confidence) ? decision.confidence : null
  };
}

async function getSettings() {
  return new Promise((resolve) => {
    chrome.storage.local.get(
      { daemonOrigin: DEFAULT_DAEMON_ORIGIN },
      (settings) => {
        resolve({
          daemonOrigin: normalizeOrigin(settings.daemonOrigin)
        });
      }
    );
  });
}

async function postJson(origin, path, body) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);

  try {
    const response = await fetch(`${origin}${path}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json"
      },
      body: JSON.stringify(body),
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Daemon returned HTTP ${response.status}.`);
    }

    return await response.json();
  } finally {
    clearTimeout(timeout);
  }
}

function normalizeOrigin(value) {
  const origin = stringOrEmpty(value).trim().replace(/\/+$/, "");

  try {
    const url = new URL(origin || DEFAULT_DAEMON_ORIGIN);
    if (url.protocol !== "http:") {
      return DEFAULT_DAEMON_ORIGIN;
    }
    if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
      return DEFAULT_DAEMON_ORIGIN;
    }
    return url.origin;
  } catch (_error) {
    return DEFAULT_DAEMON_ORIGIN;
  }
}

function stringOrEmpty(value) {
  return typeof value === "string" ? value : "";
}

function stringOrNull(value) {
  return typeof value === "string" && value.length > 0 ? value : null;
}
