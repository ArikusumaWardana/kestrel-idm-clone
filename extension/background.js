const DEFAULT_PORT = 17890;

let socket = null;
let wsConnected = false;
let altKeyBypass = false;
let reconnectTimer = null;
let keepaliveTimer = null;
let cachedConfig = {
  port: DEFAULT_PORT,
  token: "",
  interceptEnabled: true,
  bypassOnce: false,
};

async function refreshConfig() {
  cachedConfig = await chrome.storage.local.get({
    port: DEFAULT_PORT,
    token: "",
    interceptEnabled: true,
    bypassOnce: false,
  });
  return cachedConfig;
}

chrome.storage.onChanged.addListener((changes, areaName) => {
  if (areaName !== "local") return;
  for (const [key, { newValue }] of Object.entries(changes)) {
    cachedConfig[key] = newValue;
  }
});

async function getConfig() {
  return refreshConfig();
}

async function setStatus(connected, lastError = "") {
  wsConnected = connected;
  await chrome.storage.local.set({ connected, lastError });
  chrome.action.setBadgeText({ text: connected ? "" : "OFF" });
  chrome.action.setBadgeBackgroundColor({
    color: connected ? "#0f6cbd" : "#6b7280",
  });
}

function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    refreshConfig().then(() => connectWebSocket());
  }, 4000);
}

async function connectWebSocket() {
  const { port, token } = await getConfig();
  if (!token) {
    await setStatus(false, "Pair token not set");
    return;
  }

  if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
    return;
  }

  try {
    const wsUrl = `ws://127.0.0.1:${port}/ws?token=${encodeURIComponent(token)}`;
    socket = new WebSocket(wsUrl);

    socket.addEventListener("open", async () => {
      await setStatus(true, "");
      if (keepaliveTimer) clearInterval(keepaliveTimer);
      keepaliveTimer = setInterval(() => {
        if (socket && socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ type: "ping" }));
        }
      }, 15000);
    });

    socket.addEventListener("message", async (event) => {
      try {
        const msg = JSON.parse(event.data);
        if (msg.type === "refresh_link_request") {
          await handleRefreshLink(msg);
        }
      } catch {
        // Ignore malformed messages
      }
    });

    socket.addEventListener("close", async () => {
      if (keepaliveTimer) {
        clearInterval(keepaliveTimer);
        keepaliveTimer = null;
      }
      socket = null;
      await setStatus(false, "Desktop app offline");
      scheduleReconnect();
    });

    socket.addEventListener("error", async () => {
      await setStatus(false, "Connection failed");
    });
  } catch (err) {
    await setStatus(false, String(err));
    scheduleReconnect();
  }
}

async function collectCookies(targetUrl) {
  try {
    const cookies = await chrome.cookies.getAll({ url: targetUrl });
    if (!cookies || cookies.length === 0) return null;
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return null;
  }
}

async function handleRefreshLink(msg) {
  const targetUrl = msg.referer || msg.url;
  const cookies = await collectCookies(msg.url) || (msg.referer ? await collectCookies(msg.referer) : null);

  if (msg.referer) {
    try {
      const tabs = await chrome.tabs.query({ url: msg.referer.split("#")[0] + "*" });
      if (tabs.length > 0 && tabs[0].id !== undefined) {
        await chrome.tabs.reload(tabs[0].id);
      }
    } catch {
      // Tab reload is best-effort
    }
  }

  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(
      JSON.stringify({
        type: "refresh_link_response",
        download_id: msg.download_id,
        new_url: targetUrl === msg.referer ? msg.url : targetUrl,
        cookies,
        user_agent: navigator.userAgent,
        referer: msg.referer || null,
      })
    );
  }
}

chrome.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (!url || (!url.startsWith("http://") && !url.startsWith("https://"))) {
    return;
  }

  const config = cachedConfig.token ? cachedConfig : await getConfig();
  if (!config.interceptEnabled || !config.token || !wsConnected || altKeyBypass) {
    return;
  }

  if (config.bypassOnce) {
    cachedConfig.bypassOnce = false;
    await chrome.storage.local.set({ bypassOnce: false });
    return;
  }

  // Cancel immediately before async cookie/fetch calls so the browser does not
  // linger on a Save As prompt while waiting for the localhost bridge.
  try {
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch {
    // Ignore if already cancelled or cannot be erased
  }

  const cookies = await collectCookies(url);
  const payload = {
    url,
    filename: item.filename ? item.filename.split(/[/\\]/).pop() : null,
    cookies,
    userAgent: navigator.userAgent,
    referer: item.referrer || null,
  };

  try {
    const res = await fetch(`http://127.0.0.1:${config.port}/api/downloads`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-IDM-Token": config.token,
      },
      body: JSON.stringify(payload),
    });

    if (!res.ok) {
      cachedConfig.bypassOnce = true;
      await chrome.storage.local.set({ bypassOnce: true });
      await chrome.downloads.download({ url });
    }
  } catch {
    try {
      cachedConfig.bypassOnce = true;
      await chrome.storage.local.set({ bypassOnce: true });
      await chrome.downloads.download({ url });
    } catch {
      // Browser handles fallback
    }
  }
});

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "alt_state") {
    altKeyBypass = Boolean(message.pressed);
    return false;
  }
  if (message?.type === "reconnect") {
    if (socket) {
      socket.close();
      socket = null;
    }
    connectWebSocket().then(() => sendResponse({ ok: true }));
    return true;
  }
  return false;
});

chrome.runtime.onStartup.addListener(() => {
  refreshConfig().then(() => connectWebSocket());
});

chrome.runtime.onInstalled.addListener(() => {
  refreshConfig().then(() => connectWebSocket());
});

refreshConfig().then(() => connectWebSocket());
