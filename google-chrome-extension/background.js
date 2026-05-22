const DEFAULT_DAEMON_ORIGIN = "http://127.0.0.1:17891";
const ANALYZE_PATH = "/v1/dom/analyze";
const REQUEST_TIMEOUT_MS = 20000;
const MAX_ELEMENTS_PER_REQUEST = 16;

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || message.type !== "ownweb:analyzeDom") {
    return false;
  }

  analyzeDom(message)
    .then((commands) => sendResponse({ ok: true, commands }))
    .catch((error) => {
      sendResponse({
        ok: false,
        error: error instanceof Error ? error.message : String(error)
      });
    });

  return true;
});

async function analyzeDom(message) {
  const elements = Array.isArray(message.elements)
    ? message.elements.slice(0, MAX_ELEMENTS_PER_REQUEST).map(normalizeElement)
    : [];
  if (elements.length === 0) {
    return [];
  }

  const settings = await getSettings();
  const response = await postJson(settings.daemonOrigin, ANALYZE_PATH, {
    page: normalizePage(message.page),
    elements
  });

  if (!Array.isArray(response.commands)) {
    throw new Error("Daemon response must include a commands array.");
  }

  return response.commands.map(normalizeCommand);
}

function normalizePage(page) {
  return {
    url: stringOrEmpty(page && page.url),
    title: stringOrNull(page && page.title),
    capturedAt: stringOrNull(page && page.capturedAt)
  };
}

function normalizeElement(element) {
  return {
    clientId: stringOrEmpty(element.clientId),
    selector: stringOrNull(element.selector),
    tagName: stringOrNull(element.tagName),
    role: stringOrNull(element.role),
    text: stringOrEmpty(element.text),
    html: stringOrNull(element.html),
    attributes: Array.isArray(element.attributes)
      ? element.attributes.map(normalizeAttribute)
      : [],
    links: Array.isArray(element.links) ? element.links.map(normalizeLink) : [],
    snapshotHash: stringOrNull(element.snapshotHash),
    capturedAt: stringOrNull(element.capturedAt)
  };
}

function normalizeAttribute(attribute) {
  return {
    name: stringOrEmpty(attribute && attribute.name),
    value: stringOrEmpty(attribute && attribute.value)
  };
}

function normalizeLink(link) {
  return {
    href: stringOrEmpty(link && link.href),
    text: stringOrNull(link && link.text),
    ariaLabel: stringOrNull(link && link.ariaLabel)
  };
}

function normalizeCommand(command) {
  const action = stringOrEmpty(command.action);
  const allowedActions = new Set(["keep", "hide", "dim", "insertLabel", "replaceText"]);
  const target = command.target && typeof command.target === "object" ? command.target : {};

  return {
    action: allowedActions.has(action) ? action : "keep",
    target: {
      clientId: stringOrEmpty(target.clientId),
      selector: stringOrNull(target.selector),
      mustMatchSnapshotHash: stringOrNull(target.mustMatchSnapshotHash)
    },
    label: stringOrNull(command.label),
    text: stringOrNull(command.text),
    reason: stringOrNull(command.reason),
    confidence: Number.isFinite(command.confidence) ? command.confidence : null
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
