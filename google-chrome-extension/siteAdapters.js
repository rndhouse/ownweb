(() => {
  const X_COM_HOSTS = new Set(["x.com", "www.x.com", "twitter.com", "www.twitter.com"]);
  const X_COM_POST_SELECTOR = "article[data-testid='tweet']";
  const X_COM_STATUS_PATH = /^\/[A-Za-z0-9_]{1,15}\/status\/\d+(?:\/.*)?$/;

  function current(locationValue = window.location, root = document) {
    let url;
    try {
      url = new URL(locationValue.href);
    } catch (_error) {
      return null;
    }

    const xComPageKind = supportedXComPageKind(url);
    if (!xComPageKind) {
      return null;
    }

    return {
      id: "x.com",
      pageKind: xComPageKind,
      key: `x.com:${xComPageKind}:${url.origin}${url.pathname}${url.search}`,
      collectCandidates: () => collectXComCandidates(root),
      isSupportedElement: isSupportedXComElement
    };
  }

  function supportedXComPageKind(url) {
    if (!X_COM_HOSTS.has(url.hostname.toLowerCase())) {
      return null;
    }

    const path = normalizedPath(url.pathname);
    if (path === "/home") {
      return "homeTimeline";
    }
    if (path === "/search") {
      return "searchResults";
    }
    if (path === "/explore") {
      return "exploreTimeline";
    }
    if (X_COM_STATUS_PATH.test(path)) {
      return "statusThread";
    }

    return null;
  }

  function collectXComCandidates(root) {
    return Array.from(root.querySelectorAll(`main ${X_COM_POST_SELECTOR}`));
  }

  function isSupportedXComElement(element) {
    return (
      element instanceof Element &&
      element.matches(X_COM_POST_SELECTOR) &&
      element.closest("main") !== null &&
      !hasAncestorPost(element)
    );
  }

  function hasAncestorPost(element) {
    return (
      element.parentElement !== null &&
      element.parentElement.closest(X_COM_POST_SELECTOR) !== null
    );
  }

  function normalizedPath(pathname) {
    const path = String(pathname || "/").replace(/\/+$/, "");
    return path || "/";
  }

  window.WebLayerSiteAdapters = { current };
})();
