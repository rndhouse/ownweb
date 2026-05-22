const REGION_SELECTOR = "article, [role='article'], [role='listitem'], main section";
const SCAN_DEBOUNCE_MS = 250;
const MAX_REGIONS_PER_SCAN = 16;
const MAX_TEXT_CHARS = 20000;
const MAX_HTML_CHARS = 60000;
const MAX_LINKS = 80;
const MAX_ATTRIBUTES = 40;

let nextGeneratedId = 1;
let scanTimer = null;
let requestInFlight = false;

const elementIds = new WeakMap();
const elementsByClientId = new Map();
const snapshotsByClientId = new Map();
const queuedSnapshots = [];

scheduleScan();

const observer = new MutationObserver(() => {
  scheduleScan();
});

observer.observe(document.documentElement, {
  childList: true,
  subtree: true,
  characterData: true
});

document.addEventListener("click", handleOwnWebClick, true);

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || message.type !== "ownweb:applyCommands") {
    return false;
  }

  applyCommands(Array.isArray(message.commands) ? message.commands : []);
  sendResponse({ ok: true });
  return false;
});

function scheduleScan() {
  if (scanTimer !== null) {
    return;
  }

  scanTimer = setTimeout(() => {
    scanTimer = null;
    scanForRegions();
  }, SCAN_DEBOUNCE_MS);
}

function scanForRegions() {
  for (const element of collectRegions()) {
    const snapshot = snapshotElement(element);
    if (!snapshot) {
      continue;
    }

    if (element.dataset.ownwebSnapshotHash === snapshot.snapshotHash) {
      continue;
    }

    element.dataset.ownwebSnapshotHash = snapshot.snapshotHash;
    element.dataset.ownwebState = "queued";
    elementsByClientId.set(snapshot.clientId, element);
    snapshotsByClientId.set(snapshot.clientId, snapshot);
    queuedSnapshots.push(snapshot);
  }

  flushQueue();
}

function collectRegions() {
  const candidates = Array.from(document.querySelectorAll(REGION_SELECTOR))
    .filter(isVisibleRegion)
    .filter(hasSnapshotContent)
    .sort((left, right) => regionArea(left) - regionArea(right));
  const selected = [];

  for (const candidate of candidates) {
    if (selected.some((element) => element.contains(candidate) || candidate.contains(element))) {
      continue;
    }

    selected.push(candidate);
    if (selected.length >= MAX_REGIONS_PER_SCAN) {
      break;
    }
  }

  if (selected.length === 0) {
    const fallback = document.querySelector("main") || document.body;
    if (fallback && isVisibleRegion(fallback) && hasSnapshotContent(fallback)) {
      selected.push(fallback);
    }
  }

  return selected.sort((left, right) => {
    const position = left.compareDocumentPosition(right);
    return position & Node.DOCUMENT_POSITION_PRECEDING ? 1 : -1;
  });
}

function isVisibleRegion(element) {
  if (!(element instanceof Element)) {
    return false;
  }

  const rect = element.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) {
    return false;
  }
  if (rect.bottom < 0 || rect.top > window.innerHeight * 1.5) {
    return false;
  }

  const style = getComputedStyle(element);
  return style.display !== "none" && style.visibility !== "hidden" && style.opacity !== "0";
}

function hasSnapshotContent(element) {
  const clone = cloneForSnapshot(element);
  const text = normalizeText(clone.innerText || clone.textContent || "");
  return text.length > 0 || clone.querySelector("a[href]") !== null;
}

function regionArea(element) {
  const rect = element.getBoundingClientRect();
  return rect.width * rect.height;
}

function snapshotElement(element) {
  const clone = cloneForSnapshot(element);
  const text = truncate(normalizeText(clone.innerText || clone.textContent || ""), MAX_TEXT_CHARS);
  const links = snapshotLinks(clone);

  if (!text && links.length === 0) {
    return null;
  }

  const clientId = getClientId(element);
  const html = truncate(clone.outerHTML || "", MAX_HTML_CHARS);
  const attributes = snapshotAttributes(element);
  const selector = cssPath(element);
  const snapshotHash = stableHash(
    JSON.stringify({
      url: location.href,
      selector,
      text,
      links: links.map((link) => link.href)
    })
  );

  return {
    clientId,
    selector,
    tagName: element.tagName.toLowerCase(),
    role: element.getAttribute("role"),
    text,
    html,
    attributes,
    links,
    snapshotHash,
    capturedAt: new Date().toISOString()
  };
}

function cloneForSnapshot(element) {
  const clone = element.cloneNode(true);
  for (const ownwebElement of clone.querySelectorAll(".ownweb-badge")) {
    ownwebElement.remove();
  }
  for (const ownwebElement of clone.querySelectorAll("[data-ownweb-ui='true']")) {
    ownwebElement.remove();
  }
  return clone;
}

function snapshotLinks(root) {
  return Array.from(root.querySelectorAll("a[href]"))
    .slice(0, MAX_LINKS)
    .map((anchor) => ({
      href: absoluteUrl(anchor.getAttribute("href")),
      text: stringOrNull(normalizeText(anchor.innerText || anchor.textContent || "")),
      ariaLabel: stringOrNull(anchor.getAttribute("aria-label"))
    }))
    .filter((link) => link.href.length > 0);
}

function snapshotAttributes(element) {
  return Array.from(element.attributes)
    .filter((attribute) => !attribute.name.startsWith("data-ownweb"))
    .slice(0, MAX_ATTRIBUTES)
    .map((attribute) => ({
      name: attribute.name,
      value: truncate(attribute.value, 1000)
    }));
}

function getClientId(element) {
  const existingId = elementIds.get(element);
  if (existingId) {
    return existingId;
  }

  const id = `dom:${nextGeneratedId}`;
  nextGeneratedId += 1;
  elementIds.set(element, id);
  return id;
}

function pageSnapshot() {
  return {
    url: location.href,
    title: document.title || null,
    capturedAt: new Date().toISOString()
  };
}

async function flushQueue() {
  if (requestInFlight || queuedSnapshots.length === 0) {
    return;
  }

  requestInFlight = true;
  const batch = queuedSnapshots.splice(0, MAX_REGIONS_PER_SCAN);

  try {
    for (const snapshot of batch) {
      const element = elementsByClientId.get(snapshot.clientId);
      if (element) {
        element.dataset.ownwebState = "pending";
      }
    }

    const response = await sendMessage({
      type: "ownweb:analyzeDom",
      page: pageSnapshot(),
      elements: batch
    });

    if (!response || !response.ok) {
      throw new Error(response && response.error ? response.error : "Daemon request failed.");
    }

    applyCommands(response.commands || []);
  } catch (error) {
    markBatchUnavailable(batch, error);
  } finally {
    requestInFlight = false;
    if (queuedSnapshots.length > 0) {
      setTimeout(flushQueue, 100);
    }
  }
}

function sendMessage(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(message, (response) => {
      const error = chrome.runtime.lastError;
      if (error) {
        reject(new Error(error.message));
        return;
      }

      resolve(response);
    });
  });
}

function applyCommands(commands) {
  for (const command of commands) {
    const element = resolveTarget(command.target);
    if (!element || !targetStillMatches(element, command.target)) {
      continue;
    }

    if (shouldKeepFeedbackHidden(element, command)) {
      continue;
    }

    if (command.action === "insertFeedbackControl") {
      insertFeedbackControl(element, command);
      continue;
    }

    clearOwnWebChanges(element);
    element.dataset.ownwebState = command.action || "keep";

    if (command.action === "keep") {
      continue;
    }

    if (command.action === "hide") {
      element.classList.add("ownweb-hidden");
      continue;
    }

    if (command.action === "dim") {
      element.classList.add("ownweb-dimmed");
      insertBadge(element, command);
      continue;
    }

    if (command.action === "insertLabel") {
      insertBadge(element, command);
      continue;
    }

    if (command.action === "replaceText" && command.text) {
      replaceRegionText(element, command.text);
      insertBadge(element, command);
    }
  }
}

function handleOwnWebClick(event) {
  const target = eventTargetElement(event);
  const button = target ? target.closest(".ownweb-feedback-button") : null;
  if (!button) {
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  event.stopImmediatePropagation();

  const clientId = button.dataset.ownwebClientId || "";
  const element = elementsByClientId.get(clientId);
  if (!element) {
    return;
  }

  void sendFeedback(element, button);
}

function eventTargetElement(event) {
  if (event.target instanceof Element) {
    return event.target;
  }

  return event.target && event.target.parentElement instanceof Element
    ? event.target.parentElement
    : null;
}

async function sendFeedback(element, button) {
  const snapshot = snapshotElement(element);
  if (!snapshot) {
    return;
  }

  if (snapshot.snapshotHash) {
    element.dataset.ownwebUserHiddenSnapshotHash = snapshot.snapshotHash;
  }
  button.disabled = true;
  button.dataset.ownwebFeedbackState = "pending";

  try {
    const response = await sendMessage({
      type: "ownweb:feedback",
      feedback: "thumbsDown",
      page: pageSnapshot(),
      element: snapshot
    });

    if (!response || !response.ok) {
      throw new Error(response && response.error ? response.error : "Daemon request failed.");
    }

    applyCommands(response.commands || []);
  } catch (error) {
    delete element.dataset.ownwebUserHiddenSnapshotHash;
    button.disabled = false;
    button.dataset.ownwebFeedbackState = "unavailable";
    button.title = `OwnWeb feedback unavailable: ${
      error instanceof Error ? error.message : String(error)
    }`;
  }
}

function shouldKeepFeedbackHidden(element, command) {
  const hiddenSnapshotHash = element.dataset.ownwebUserHiddenSnapshotHash;
  if (!hiddenSnapshotHash || command.action === "hide") {
    return false;
  }

  const commandSnapshotHash = command.target && command.target.mustMatchSnapshotHash;
  return !commandSnapshotHash || commandSnapshotHash === hiddenSnapshotHash;
}

function resolveTarget(target) {
  if (!target || typeof target !== "object") {
    return null;
  }

  if (target.clientId && elementsByClientId.has(target.clientId)) {
    return elementsByClientId.get(target.clientId);
  }

  if (target.selector) {
    try {
      const element = document.querySelector(target.selector);
      if (element) {
        return element;
      }
    } catch (_error) {
      return null;
    }
  }

  return null;
}

function targetStillMatches(element, target) {
  if (!target || !target.mustMatchSnapshotHash) {
    return true;
  }

  const snapshot = snapshotElement(element);
  return snapshot && snapshot.snapshotHash === target.mustMatchSnapshotHash;
}

function clearOwnWebChanges(element) {
  element.classList.remove("ownweb-hidden", "ownweb-dimmed", "ownweb-replaced");

  const badge = element.querySelector(":scope > .ownweb-badge");
  if (badge) {
    badge.remove();
  }

  if (element.dataset.ownwebOriginalText) {
    element.innerText = element.dataset.ownwebOriginalText;
    delete element.dataset.ownwebOriginalText;
  }
}

function replaceRegionText(element, replacementText) {
  element.dataset.ownwebOriginalText = element.innerText;
  element.innerText = replacementText;
  element.classList.add("ownweb-replaced");
}

function insertBadge(element, command) {
  const badgeText = command.label || command.reason || "OwnWeb";
  const badge = document.createElement("div");
  const text = document.createElement("span");

  badge.className = "ownweb-badge";
  badge.dataset.ownwebUi = "true";

  text.className = "ownweb-badge-text";
  text.dataset.ownwebUi = "true";
  text.textContent = badgeText;

  badge.append(text);
  element.prepend(badge);
}

function insertFeedbackControl(element, command) {
  const clientId = command.target && command.target.clientId
    ? command.target.clientId
    : getClientId(element);
  const isSubjectPost = isSubjectPostElement(element, clientId);
  const existingButton = element.querySelector(
    `.ownweb-feedback-button[data-ownweb-client-id="${cssEscape(clientId)}"]`
  );
  if (existingButton) {
    existingButton.classList.toggle("ownweb-feedback-button--subject", isSubjectPost);
    return;
  }

  const actionBar = findActionBar(element);
  if (!actionBar) {
    return;
  }

  const likeSlot = findActionSlot(actionBar, "[data-testid='like'], [data-testid='unlike']");
  const slot = createFeedbackSlot(likeSlot || actionBar.firstElementChild);
  slot.dataset.ownwebUi = "true";
  slot.append(createFeedbackButton(clientId, command.label || "Hide this post", isSubjectPost));

  if (likeSlot && likeSlot.parentElement === actionBar && likeSlot.nextSibling) {
    actionBar.insertBefore(slot, likeSlot.nextSibling);
  } else {
    actionBar.append(slot);
  }
}

function createFeedbackSlot(referenceSlot) {
  const slot = document.createElement(
    referenceSlot && referenceSlot.tagName
      ? referenceSlot.tagName.toLowerCase()
      : "div"
  );
  const referenceClass = referenceSlot && typeof referenceSlot.className === "string"
    ? referenceSlot.className
    : "";
  slot.className = referenceClass
    ? `${referenceClass} ownweb-feedback-slot`
    : "ownweb-feedback-slot";
  return slot;
}

function createFeedbackButton(clientId, label, isSubjectPost) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "ownweb-feedback-button";
  button.classList.toggle("ownweb-feedback-button--subject", isSubjectPost);
  button.dataset.ownwebUi = "true";
  button.dataset.ownwebClientId = clientId;
  button.dataset.ownwebFeedback = "thumbsDown";
  button.title = label;
  button.setAttribute("aria-label", label);
  button.append(createThumbsDownIcon());
  return button;
}

function isSubjectPostElement(element, clientId) {
  const pageStatusId = statusIdFromUrl(location.href);
  if (!pageStatusId) {
    return false;
  }

  const snapshot = snapshotsByClientId.get(clientId);
  if (snapshot && snapshot.links.some((link) => statusIdFromUrl(link.href) === pageStatusId)) {
    return true;
  }

  const postRoot = element.closest("article, [role='article'], [data-testid='tweet']");
  const firstPost = firstVisiblePostInMain();
  return postRoot !== null && postRoot === firstPost;
}

function firstVisiblePostInMain() {
  const main = document.querySelector("main");
  if (!main) {
    return null;
  }

  return Array.from(main.querySelectorAll("article, [role='article'], [data-testid='tweet']"))
    .filter(isVisibleRegion)[0] || null;
}

function statusIdFromUrl(value) {
  const match = String(value || "").match(/\/status\/(\d+)/);
  return match ? match[1] : null;
}

function createThumbsDownIcon() {
  const namespace = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(namespace, "svg");
  const path = document.createElementNS(namespace, "path");

  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", "ownweb-feedback-icon");
  path.setAttribute(
    "d",
    "M10 15v4a3 3 0 0 0 3 3l4-9V2H5.7a2 2 0 0 0-2 1.7l-1.4 9A2 2 0 0 0 4.3 15H10Zm7-13h2.7A2.3 2.3 0 0 1 22 4.3v6.4a2.3 2.3 0 0 1-2.3 2.3H17V2Z"
  );
  svg.append(path);
  return svg;
}

function findActionBar(element) {
  const candidates = Array.from(element.querySelectorAll("[role='group']"))
    .filter(isVisibleRegion)
    .filter((candidate) => candidate.querySelectorAll("button, [role='button']").length >= 2)
    .sort((left, right) => {
      const leftRect = left.getBoundingClientRect();
      const rightRect = right.getBoundingClientRect();
      return rightRect.top - leftRect.top;
    });

  return candidates[0] || null;
}

function findActionSlot(actionBar, selector) {
  const control = actionBar.querySelector(selector);
  if (!control) {
    return null;
  }

  let current = control;
  while (current.parentElement && current.parentElement !== actionBar) {
    current = current.parentElement;
  }

  return current.parentElement === actionBar ? current : null;
}

function markBatchUnavailable(batch, error) {
  for (const snapshot of batch) {
    const element = elementsByClientId.get(snapshot.clientId);
    if (!element) {
      continue;
    }

    element.dataset.ownwebState = "unavailable";
    element.title = `OwnWeb daemon unavailable: ${
      error instanceof Error ? error.message : String(error)
    }`;
  }
}

function cssPath(element) {
  const parts = [];
  let current = element;

  while (current && current.nodeType === Node.ELEMENT_NODE && current !== document.documentElement) {
    const tag = current.tagName.toLowerCase();
    const id = current.getAttribute("id");
    if (id) {
      parts.unshift(`${tag}#${cssEscape(id)}`);
      break;
    }

    let index = 1;
    let sibling = current.previousElementSibling;
    while (sibling) {
      if (sibling.tagName === current.tagName) {
        index += 1;
      }
      sibling = sibling.previousElementSibling;
    }

    parts.unshift(`${tag}:nth-of-type(${index})`);
    current = current.parentElement;
  }

  return parts.length > 0 ? parts.join(" > ") : null;
}

function cssEscape(value) {
  if (window.CSS && typeof window.CSS.escape === "function") {
    return window.CSS.escape(value);
  }

  return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
}

function absoluteUrl(value) {
  if (!value) {
    return "";
  }

  try {
    return new URL(value, location.href).href;
  } catch (_error) {
    return "";
  }
}

function normalizeText(text) {
  return String(text || "").replace(/\s+/g, " ").trim();
}

function truncate(value, maxLength) {
  const stringValue = String(value || "");
  return stringValue.length > maxLength ? stringValue.slice(0, maxLength) : stringValue;
}

function stringOrNull(value) {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function stableHash(value) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}
