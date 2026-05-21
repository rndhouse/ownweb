const DEFAULT_DAEMON_ORIGIN = "http://127.0.0.1:17891";

const daemonOriginInput = document.querySelector("#daemonOrigin");
const saveButton = document.querySelector("#saveButton");
const testButton = document.querySelector("#testButton");
const statusElement = document.querySelector("#status");

loadOptions();

saveButton.addEventListener("click", saveOptions);
testButton.addEventListener("click", testDaemon);

function loadOptions() {
  chrome.storage.local.get({ daemonOrigin: DEFAULT_DAEMON_ORIGIN }, (settings) => {
    daemonOriginInput.value = normalizeOrigin(settings.daemonOrigin);
  });
}

function saveOptions() {
  const daemonOrigin = normalizeOrigin(daemonOriginInput.value);
  daemonOriginInput.value = daemonOrigin;

  chrome.storage.local.set({ daemonOrigin }, () => {
    setStatus("Saved.");
  });
}

async function testDaemon() {
  const daemonOrigin = normalizeOrigin(daemonOriginInput.value);
  daemonOriginInput.value = daemonOrigin;
  setStatus("Testing...");

  try {
    const response = await fetch(`${daemonOrigin}/health`);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    setStatus("Daemon is reachable.");
  } catch (error) {
    setStatus(`Daemon is not reachable: ${error instanceof Error ? error.message : error}`);
  }
}

function normalizeOrigin(value) {
  const origin = String(value || "").trim().replace(/\/+$/, "");

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

function setStatus(message) {
  statusElement.textContent = message;
}
