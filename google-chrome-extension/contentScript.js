const REGION_SELECTOR = "article, [role='article'], [role='listitem'], main section";
const SCAN_DEBOUNCE_MS = 250;
const MAX_REGIONS_PER_SCAN = 16;
const MAX_TEXT_CHARS = 20000;
const MAX_HTML_CHARS = 60000;
const MAX_LINKS = 80;
const MAX_ATTRIBUTES = 40;
const FEEDBACK_REASON_SAVE_DEBOUNCE_MS = 550;
const FEEDBACK_REASON_PRESETS = [
  "Low information",
  "Rage bait",
  "Spam",
  "AI slop",
  "Not interested"
];

let nextGeneratedId = 1;
let nextFeedbackSaveId = 1;
let scanTimer = null;
let requestInFlight = false;

const elementIds = new WeakMap();
const elementsByClientId = new Map();
const snapshotsByClientId = new Map();
const feedbackReasonTimers = new WeakMap();
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

document.addEventListener("click", handleWebLayerClick, true);
document.addEventListener("pointerdown", handleWebLayerPointerDown, true);

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || message.type !== "weblayer:applyCommands") {
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

    if (element.dataset.weblayerSnapshotHash === snapshot.snapshotHash) {
      continue;
    }

    if (
      element.dataset.weblayerKeepVisibleAfterFeedbackHash &&
      element.dataset.weblayerKeepVisibleAfterFeedbackHash !== snapshot.snapshotHash
    ) {
      delete element.dataset.weblayerKeepVisibleAfterFeedbackHash;
    }

    element.dataset.weblayerSnapshotHash = snapshot.snapshotHash;
    element.dataset.weblayerState = "queued";
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
  for (const weblayerElement of clone.querySelectorAll(".weblayer-badge")) {
    weblayerElement.remove();
  }
  for (const weblayerElement of clone.querySelectorAll("[data-weblayer-ui='true']")) {
    weblayerElement.remove();
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
    .filter((attribute) => !attribute.name.startsWith("data-weblayer"))
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
        element.dataset.weblayerState = "pending";
      }
    }

    const response = await sendMessage({
      type: "weblayer:analyzeDom",
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

    if (command.action === "insertFeedbackControl") {
      insertFeedbackControl(element, command);
      continue;
    }

    if (command.action === "hide" && shouldKeepVisibleAfterFeedback(element, command)) {
      element.dataset.weblayerState = "feedbackActive";
      continue;
    }

    clearWebLayerChanges(element);
    element.dataset.weblayerState = command.action || "keep";

    if (command.action === "keep") {
      continue;
    }

    if (command.action === "hide") {
      element.classList.add("weblayer-hidden");
      continue;
    }

    if (command.action === "dim") {
      element.classList.add("weblayer-dimmed");
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

function handleWebLayerClick(event) {
  const target = eventTargetElement(event);
  const button = target ? target.closest(".weblayer-feedback-button") : null;
  if (!button) {
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  event.stopImmediatePropagation();

  const clientId = button.dataset.weblayerClientId || "";
  const element = elementsByClientId.get(clientId);
  if (!element || button.disabled) {
    return;
  }

  void toggleFeedback(element, button);
}

function handleWebLayerPointerDown(event) {
  const target = eventTargetElement(event);
  const button = target ? target.closest(".weblayer-feedback-button") : null;
  if (!button || !button.classList.contains("weblayer-feedback-button--active")) {
    return;
  }

  button.dataset.weblayerSkipNextReasonBlur = "true";
  setTimeout(() => {
    delete button.dataset.weblayerSkipNextReasonBlur;
  }, 300);
}

function eventTargetElement(event) {
  if (event.target instanceof Element) {
    return event.target;
  }

  return event.target && event.target.parentElement instanceof Element
    ? event.target.parentElement
    : null;
}

async function toggleFeedback(element, button) {
  const wasActive = button.classList.contains("weblayer-feedback-button--active");

  if (!wasActive) {
    markKeepVisibleAfterFeedback(element);
    button.dataset.weblayerFeedbackPersisted = "false";
    setFeedbackButtonActive(button, true);
    showFeedbackReasonPanel(element, button);
    return;
  }

  const panel = wasActive
    ? feedbackReasonPanel(element, button.dataset.weblayerClientId || "")
    : null;
  const persisted = button.dataset.weblayerFeedbackPersisted === "true";
  const reason = currentFeedbackReason(element, button);

  if (panel) {
    cancelScheduledReasonUpdate(panel);
    panel.dataset.weblayerClosing = "true";
  }

  button.disabled = true;
  button.dataset.weblayerFeedbackState = "pending";

  try {
    if (persisted) {
      const response = await sendFeedbackEvent(element, "undoThumbsDown", reason, button);

      if (!response || !response.ok) {
        throw new Error(response && response.error ? response.error : "Daemon request failed.");
      }

      applyCommands(response.commands || []);
    }

    clearKeepVisibleAfterFeedback(element);
    delete button.dataset.weblayerFeedbackPersisted;
    setFeedbackButtonActive(button, false);
    removeFeedbackReasonPanel(element, button.dataset.weblayerClientId || "");
  } catch (error) {
    if (panel) {
      delete panel.dataset.weblayerClosing;
      setFeedbackSaveStatus(panel, "Undo failed", "error");
    }
    button.dataset.weblayerFeedbackState = wasActive ? "active" : "unavailable";
    button.title = `WebLayer feedback unavailable: ${
      error instanceof Error ? error.message : String(error)
    }`;
  } finally {
    button.disabled = false;
  }
}

async function sendFeedbackEvent(element, feedback, reason, button) {
  const snapshot = snapshotElement(element);
  if (!snapshot) {
    return null;
  }
  const feedbackContext = currentFeedbackContext(element, snapshot.clientId, button);
  const message = {
    type: "weblayer:feedback",
    feedback,
    reason,
    page: pageSnapshot(),
    element: snapshot,
    feedbackContext
  };

  return sendMessage(message);
}

function setFeedbackButtonActive(button, active) {
  const label = active ? "Undo thumbs-down feedback" : "Hide this post";
  button.classList.toggle("weblayer-feedback-button--active", active);
  button.dataset.weblayerFeedbackState = active ? "active" : "idle";
  button.title = label;
  button.setAttribute("aria-label", label);
  button.setAttribute("aria-pressed", active ? "true" : "false");
}

function markKeepVisibleAfterFeedback(element) {
  const snapshot = snapshotElement(element);
  if (snapshot && snapshot.snapshotHash) {
    element.dataset.weblayerKeepVisibleAfterFeedbackHash = snapshot.snapshotHash;
  }
}

function clearKeepVisibleAfterFeedback(element) {
  delete element.dataset.weblayerKeepVisibleAfterFeedbackHash;
}

function shouldKeepVisibleAfterFeedback(element, command) {
  if (hasActiveFeedbackSession(element)) {
    return true;
  }

  const keepVisibleHash = element.dataset.weblayerKeepVisibleAfterFeedbackHash;
  if (!keepVisibleHash) {
    return false;
  }

  const commandHash = command.target && command.target.mustMatchSnapshotHash;
  if (commandHash) {
    return commandHash === keepVisibleHash;
  }

  const snapshot = snapshotElement(element);
  return snapshot && snapshot.snapshotHash === keepVisibleHash;
}

function hasActiveFeedbackSession(element) {
  return (
    element.querySelector(".weblayer-feedback-panel") !== null ||
    element.querySelector(".weblayer-feedback-button--active") !== null
  );
}

function showFeedbackReasonPanel(element, button) {
  const clientId = button.dataset.weblayerClientId || "";
  if (!clientId) {
    return;
  }

  const existingPanel = feedbackReasonPanel(element, clientId);
  if (existingPanel) {
    const input = existingPanel.querySelector(".weblayer-feedback-reason-input");
    if (input instanceof HTMLElement) {
      input.focus();
    }
    return;
  }

  const actionBar = findActionBar(element);
  if (!actionBar) {
    return;
  }

  const panel = createFeedbackReasonPanel(element, button);
  actionBar.insertAdjacentElement("afterend", panel);

  const input = panel.querySelector(".weblayer-feedback-reason-input");
  if (input instanceof HTMLElement) {
    input.focus();
  }
}

function createFeedbackReasonPanel(element, button) {
  const clientId = button.dataset.weblayerClientId || "";
  const panel = document.createElement("div");
  const label = document.createElement("div");
  const status = document.createElement("div");
  const chips = document.createElement("div");
  const input = document.createElement("textarea");

  panel.className = "weblayer-feedback-panel";
  panel.dataset.weblayerUi = "true";
  panel.dataset.weblayerClientId = clientId;
  panel.dataset.weblayerSavedReason = "";
  panel.dataset.weblayerSaveState = "idle";

  label.className = "weblayer-feedback-panel-label";
  label.dataset.weblayerUi = "true";
  label.textContent = "Reason";

  status.className = "weblayer-feedback-save-status";
  status.dataset.weblayerUi = "true";
  status.setAttribute("role", "status");
  status.setAttribute("aria-live", "polite");
  status.textContent = "Add a reason";

  chips.className = "weblayer-feedback-reason-chips";
  chips.dataset.weblayerUi = "true";
  for (const reason of FEEDBACK_REASON_PRESETS) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "weblayer-feedback-reason-chip";
    chip.dataset.weblayerUi = "true";
    chip.dataset.weblayerReason = reason;
    chip.textContent = reason;
    chip.setAttribute("aria-pressed", "false");
    chip.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation();
      input.value = reason;
      input.focus();
      updateSelectedReasonChip(panel, reason);
      scheduleReasonUpdate(element, button, panel, { immediate: true });
    });
    chips.append(chip);
  }

  input.className = "weblayer-feedback-reason-input";
  input.dataset.weblayerUi = "true";
  input.rows = 2;
  input.placeholder = "Add a reason";
  input.addEventListener("click", (event) => {
    event.stopPropagation();
  });
  input.addEventListener("keydown", (event) => {
    event.stopPropagation();
  });
  input.addEventListener("input", () => {
    updateSelectedReasonChip(panel, currentPanelReason(panel));
    scheduleReasonUpdate(element, button, panel);
  });
  input.addEventListener("blur", () => {
    if (button.dataset.weblayerSkipNextReasonBlur === "true") {
      return;
    }

    scheduleReasonUpdate(element, button, panel, { immediate: true });
  });
  input.addEventListener("change", () => {
    scheduleReasonUpdate(element, button, panel, { immediate: true });
  });

  panel.addEventListener("pointerdown", stopPanelEvent);
  panel.addEventListener("click", stopPanelEvent);
  panel.addEventListener("keydown", stopPanelEvent);

  panel.append(label, status, chips, input);
  return panel;
}

function stopPanelEvent(event) {
  event.stopPropagation();
}

function scheduleReasonUpdate(element, button, panel, options = {}) {
  if (
    !button.classList.contains("weblayer-feedback-button--active") ||
    panel.dataset.weblayerClosing === "true"
  ) {
    return;
  }

  const reason = currentPanelReason(panel);
  updateSelectedReasonChip(panel, reason);
  cancelScheduledReasonUpdate(panel);

  if (!reason) {
    setFeedbackSaveStatus(panel, "Add a reason", "idle");
    return;
  }

  if ((panel.dataset.weblayerSavedReason || "") === reason) {
    setFeedbackSaveStatus(panel, "Saved", "saved");
    return;
  }

  setFeedbackSaveStatus(panel, "Saving...", "saving");

  const save = () => {
    feedbackReasonTimers.delete(panel);
    void sendReasonUpdate(element, button, panel, reason);
  };

  if (options.immediate) {
    save();
    return;
  }

  feedbackReasonTimers.set(
    panel,
    setTimeout(save, FEEDBACK_REASON_SAVE_DEBOUNCE_MS)
  );
}

function cancelScheduledReasonUpdate(panel) {
  const timer = feedbackReasonTimers.get(panel);
  if (timer) {
    clearTimeout(timer);
    feedbackReasonTimers.delete(panel);
  }
}

async function sendReasonUpdate(element, button, panel, reason) {
  if (panel.dataset.weblayerClosing === "true" || !reason) {
    return;
  }

  const requestId = String(nextFeedbackSaveId);
  nextFeedbackSaveId += 1;
  const previousTitle = button.title;
  const feedback = button.dataset.weblayerFeedbackPersisted === "true"
    ? "updateReason"
    : "thumbsDown";
  panel.dataset.weblayerSaveRequestId = requestId;
  button.disabled = true;
  button.dataset.weblayerFeedbackState = "pending";
  setFeedbackSaveStatus(panel, "Saving...", "saving");

  try {
    const response = await sendFeedbackEvent(element, feedback, reason, button);
    if (!response || !response.ok) {
      throw new Error(response && response.error ? response.error : "Daemon request failed.");
    }

    applyCommands(response.commands || []);
    if (!panel.isConnected || panel.dataset.weblayerClosing === "true") {
      return;
    }
    if (
      panel.dataset.weblayerSaveRequestId !== requestId ||
      currentPanelReason(panel) !== reason
    ) {
      return;
    }

    panel.dataset.weblayerSavedReason = reason;
    button.dataset.weblayerFeedbackPersisted = "true";
    setFeedbackSaveStatus(panel, `Saved ${shortTime(new Date())}`, "saved");
    if (button.classList.contains("weblayer-feedback-button--active")) {
      button.dataset.weblayerFeedbackState = "active";
    }
  } catch (error) {
    if (
      panel.isConnected &&
      panel.dataset.weblayerSaveRequestId === requestId &&
      panel.dataset.weblayerClosing !== "true"
    ) {
      setFeedbackSaveStatus(panel, "Save failed", "error");
    }

    button.dataset.weblayerFeedbackState = "active";
    button.title = `WebLayer feedback unavailable: ${
      error instanceof Error ? error.message : String(error)
    }`;
    setTimeout(() => {
      if (button.dataset.weblayerFeedbackState === "active") {
        button.title = previousTitle;
      }
    }, 2500);
  } finally {
    if (
      panel.isConnected &&
      panel.dataset.weblayerSaveRequestId === requestId &&
      panel.dataset.weblayerClosing !== "true"
    ) {
      button.disabled = false;
    }
  }
}

function currentPanelReason(panel) {
  const input = panel.querySelector(".weblayer-feedback-reason-input");
  return input instanceof HTMLTextAreaElement ? input.value.trim() : "";
}

function updateSelectedReasonChip(panel, reason) {
  for (const chip of panel.querySelectorAll(".weblayer-feedback-reason-chip")) {
    const selected = chip.dataset.weblayerReason === reason;
    chip.classList.toggle("weblayer-feedback-reason-chip--selected", selected);
    chip.setAttribute("aria-pressed", selected ? "true" : "false");
  }
}

function setFeedbackSaveStatus(panel, text, state) {
  panel.dataset.weblayerSaveState = state;
  const status = panel.querySelector(".weblayer-feedback-save-status");
  if (status) {
    status.textContent = text;
  }
}

function shortTime(date) {
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit"
  });
}

function currentFeedbackReason(element, button) {
  const clientId = button.dataset.weblayerClientId || "";
  const panel = feedbackReasonPanel(element, clientId);
  const input = panel && panel.querySelector(".weblayer-feedback-reason-input");
  return input instanceof HTMLTextAreaElement ? input.value.trim() : "";
}

function feedbackReasonPanel(element, clientId) {
  if (!clientId) {
    return null;
  }

  return element.querySelector(
    `.weblayer-feedback-panel[data-weblayer-client-id="${cssEscape(clientId)}"]`
  );
}

function removeFeedbackReasonPanel(element, clientId) {
  const panel = feedbackReasonPanel(element, clientId);
  if (panel) {
    cancelScheduledReasonUpdate(panel);
    panel.remove();
  }
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

function clearWebLayerChanges(element) {
  element.classList.remove("weblayer-hidden", "weblayer-dimmed", "weblayer-replaced");

  const badge = element.querySelector(":scope > .weblayer-badge");
  if (badge) {
    badge.remove();
  }

  if (element.dataset.weblayerOriginalText) {
    element.innerText = element.dataset.weblayerOriginalText;
    delete element.dataset.weblayerOriginalText;
  }
}

function replaceRegionText(element, replacementText) {
  element.dataset.weblayerOriginalText = element.innerText;
  element.innerText = replacementText;
  element.classList.add("weblayer-replaced");
}

function insertBadge(element, command) {
  const badgeText = command.label || command.reason || "WebLayer";
  const badge = document.createElement("div");
  const text = document.createElement("span");

  badge.className = "weblayer-badge";
  badge.dataset.weblayerUi = "true";

  text.className = "weblayer-badge-text";
  text.dataset.weblayerUi = "true";
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
    `.weblayer-feedback-button[data-weblayer-client-id="${cssEscape(clientId)}"]`
  );
  if (existingButton) {
    existingButton.classList.toggle("weblayer-feedback-button--subject", isSubjectPost);
    storeFeedbackContext(existingButton, command.feedbackContext);
    if (!existingButton.hasAttribute("aria-pressed")) {
      existingButton.setAttribute("aria-pressed", "false");
    }
    return;
  }

  const actionBar = findActionBar(element);
  if (!actionBar) {
    return;
  }

  const likeSlot = findActionSlot(actionBar, "[data-testid='like'], [data-testid='unlike']");
  const slot = createFeedbackSlot(likeSlot || actionBar.firstElementChild);
  slot.dataset.weblayerUi = "true";
  slot.append(
    createFeedbackButton(
      clientId,
      command.label || "Hide this post",
      isSubjectPost,
      command.feedbackContext
    )
  );

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
    ? `${referenceClass} weblayer-feedback-slot`
    : "weblayer-feedback-slot";
  return slot;
}

function createFeedbackButton(clientId, label, isSubjectPost, feedbackContext) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "weblayer-feedback-button";
  button.classList.toggle("weblayer-feedback-button--subject", isSubjectPost);
  button.dataset.weblayerUi = "true";
  button.dataset.weblayerClientId = clientId;
  button.dataset.weblayerFeedback = "thumbsDown";
  button.dataset.weblayerFeedbackState = "idle";
  button.title = label;
  button.setAttribute("aria-label", label);
  button.setAttribute("aria-pressed", "false");
  storeFeedbackContext(button, feedbackContext);
  button.append(createThumbsDownIcon());
  return button;
}

function storeFeedbackContext(button, feedbackContext) {
  if (!feedbackContext || typeof feedbackContext !== "object" || Array.isArray(feedbackContext)) {
    throw new Error("Feedback context is required.");
  }
  button.dataset.weblayerFeedbackContext = JSON.stringify(feedbackContext);
}

function currentFeedbackContext(element, clientId, button) {
  const source = button || element.querySelector(
    `.weblayer-feedback-button[data-weblayer-client-id="${cssEscape(clientId)}"]`
  );
  if (!source || !source.dataset.weblayerFeedbackContext) {
    throw new Error("Feedback context is missing.");
  }

  try {
    const feedbackContext = JSON.parse(source.dataset.weblayerFeedbackContext);
    if (
      !feedbackContext ||
      typeof feedbackContext !== "object" ||
      Array.isArray(feedbackContext)
    ) {
      throw new Error("Feedback context is invalid.");
    }
    return feedbackContext;
  } catch (_error) {
    delete source.dataset.weblayerFeedbackContext;
    throw new Error("Feedback context is invalid.");
  }
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
  svg.setAttribute("class", "weblayer-feedback-icon");
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

    element.dataset.weblayerState = "unavailable";
    element.title = `WebLayer daemon unavailable: ${
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
