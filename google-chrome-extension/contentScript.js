const TWEET_SELECTOR = 'article[data-testid="tweet"]';
const TWEET_TEXT_SELECTOR = '[data-testid="tweetText"]';
const STATUS_LINK_SELECTOR = 'a[href*="/status/"]';
const MAX_BATCH_SIZE = 4;
const SCAN_DEBOUNCE_MS = 200;

let nextGeneratedId = 1;
let scanTimer = null;
let requestInFlight = false;

const elementIds = new WeakMap();
const queuedPosts = [];
const elementsByClientId = new Map();

scheduleScan();

const observer = new MutationObserver(() => {
  scheduleScan();
});

observer.observe(document.documentElement, {
  childList: true,
  subtree: true
});

function scheduleScan() {
  if (scanTimer !== null) {
    return;
  }

  scanTimer = setTimeout(() => {
    scanTimer = null;
    scanForTweets();
  }, SCAN_DEBOUNCE_MS);
}

function scanForTweets() {
  const articles = document.querySelectorAll(TWEET_SELECTOR);

  for (const article of articles) {
    const post = extractPost(article);
    if (!post) {
      continue;
    }

    const signature = postSignature(post.url, post.text);
    if (article.dataset.pairpilotSignature === signature) {
      continue;
    }

    article.dataset.pairpilotSignature = signature;
    article.dataset.pairpilotState = "queued";
    elementsByClientId.set(post.clientId, article);
    queuedPosts.push(post);
  }

  flushQueue();
}

function extractPost(article) {
  const textElement = article.querySelector(TWEET_TEXT_SELECTOR);
  const text = normalizeText(textElement ? textElement.innerText : "");
  const statusLink = article.querySelector(STATUS_LINK_SELECTOR);
  const url = statusLink ? new URL(statusLink.getAttribute("href"), location.origin).href : null;
  const postId = extractPostId(url);
  const clientId = getClientId(article, postId);

  if (!text && !url) {
    return null;
  }

  return {
    clientId,
    postId,
    url,
    authorHandle: extractAuthorHandle(statusLink),
    text,
    capturedAt: new Date().toISOString()
  };
}

function getClientId(article, postId) {
  const existingId = elementIds.get(article);
  if (existingId) {
    return existingId;
  }

  const id = postId ? `x:${postId}:${nextGeneratedId}` : `x:generated:${nextGeneratedId}`;
  nextGeneratedId += 1;
  elementIds.set(article, id);
  return id;
}

function extractPostId(url) {
  if (!url) {
    return null;
  }

  const match = url.match(/\/status\/(\d+)/);
  return match ? match[1] : null;
}

function extractAuthorHandle(statusLink) {
  if (!statusLink) {
    return null;
  }

  const href = statusLink.getAttribute("href");
  if (!href) {
    return null;
  }

  const pathname = new URL(href, location.origin).pathname;
  const match = pathname.match(/^\/([^/]+)\/status\//);
  return match ? `@${match[1]}` : null;
}

function normalizeText(text) {
  return text.replace(/\s+/g, " ").trim();
}

function postSignature(url, text) {
  return `${url || ""}\n${text}`;
}

async function flushQueue() {
  if (requestInFlight || queuedPosts.length === 0) {
    return;
  }

  requestInFlight = true;
  const batch = queuedPosts.splice(0, MAX_BATCH_SIZE);

  try {
    for (const post of batch) {
      const element = elementsByClientId.get(post.clientId);
      if (element) {
        element.dataset.pairpilotState = "pending";
      }
    }

    const response = await sendMessage({
      type: "pairpilot:analyzePosts",
      posts: batch
    });

    if (!response || !response.ok) {
      throw new Error(response && response.error ? response.error : "Daemon request failed.");
    }

    applyDecisions(response.decisions || []);
  } catch (error) {
    markBatchUnavailable(batch, error);
  } finally {
    requestInFlight = false;
    if (queuedPosts.length > 0) {
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

function applyDecisions(decisions) {
  for (const decision of decisions) {
    const article = elementsByClientId.get(decision.clientId);
    if (!article) {
      continue;
    }

    clearPairpilotChanges(article);
    article.dataset.pairpilotState = decision.action || "keep";

    if (decision.action === "hide") {
      article.classList.add("pairpilot-hidden");
      continue;
    }

    if (decision.action === "dim") {
      article.classList.add("pairpilot-dimmed");
      insertBadge(article, decision);
      continue;
    }

    if (decision.action === "label") {
      insertBadge(article, decision);
      continue;
    }

    if (decision.action === "replace" && decision.replacementText) {
      replaceTweetText(article, decision.replacementText);
      insertBadge(article, decision);
    }
  }
}

function clearPairpilotChanges(article) {
  article.classList.remove("pairpilot-hidden", "pairpilot-dimmed", "pairpilot-replaced");

  const badge = article.querySelector(":scope > .pairpilot-badge");
  if (badge) {
    badge.remove();
  }

  const textElement = article.querySelector(TWEET_TEXT_SELECTOR);
  if (textElement && textElement.dataset.pairpilotOriginalText) {
    textElement.innerText = textElement.dataset.pairpilotOriginalText;
    delete textElement.dataset.pairpilotOriginalText;
  }
}

function replaceTweetText(article, replacementText) {
  const textElement = article.querySelector(TWEET_TEXT_SELECTOR);
  if (!textElement) {
    return;
  }

  textElement.dataset.pairpilotOriginalText = textElement.innerText;
  textElement.innerText = replacementText;
  article.dataset.pairpilotSignature = postSignature(
    currentPostUrl(article),
    normalizeText(replacementText)
  );
  article.classList.add("pairpilot-replaced");
}

function currentPostUrl(article) {
  const statusLink = article.querySelector(STATUS_LINK_SELECTOR);
  const href = statusLink ? statusLink.getAttribute("href") : null;
  return href ? new URL(href, location.origin).href : null;
}

function insertBadge(article, decision) {
  const badge = document.createElement("div");
  badge.className = "pairpilot-badge";
  badge.textContent = badgeText(decision);
  article.prepend(badge);
}

function badgeText(decision) {
  if (decision.label) {
    return decision.label;
  }

  if (decision.reason) {
    return `Pairpilot: ${decision.reason}`;
  }

  return "Pairpilot filtered this post";
}

function markBatchUnavailable(batch, error) {
  for (const post of batch) {
    const article = elementsByClientId.get(post.clientId);
    if (!article) {
      continue;
    }

    article.dataset.pairpilotState = "unavailable";
    article.title = `Pairpilot daemon unavailable: ${
      error instanceof Error ? error.message : String(error)
    }`;
  }
}
