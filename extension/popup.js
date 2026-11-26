const statusBadge = document.getElementById("status-badge");
const interceptToggle = document.getElementById("intercept-toggle");
const portInput = document.getElementById("port-input");
const tokenInput = document.getElementById("token-input");
const feedback = document.getElementById("feedback");
const saveBtn = document.getElementById("save-btn");
const bypassBtn = document.getElementById("bypass-btn");

async function loadState() {
  const data = await chrome.storage.local.get({
    port: 17890,
    token: "",
    interceptEnabled: true,
    connected: false,
    lastError: "",
  });

  portInput.value = String(data.port);
  tokenInput.value = data.token;
  interceptToggle.checked = Boolean(data.interceptEnabled);

  statusBadge.textContent = data.connected ? "Connected" : "Offline";
  statusBadge.dataset.state = data.connected ? "online" : "offline";

  if (!data.connected && data.lastError) {
    feedback.textContent = data.lastError;
  }
}

saveBtn.addEventListener("click", async () => {
  const port = Number(portInput.value) || 17890;
  const token = tokenInput.value.trim();
  const interceptEnabled = interceptToggle.checked;

  await chrome.storage.local.set({ port, token, interceptEnabled });
  feedback.textContent = "Connecting...";
  chrome.runtime.sendMessage({ type: "reconnect" }, () => {
    setTimeout(loadState, 600);
  });
});

interceptToggle.addEventListener("change", async () => {
  await chrome.storage.local.set({ interceptEnabled: interceptToggle.checked });
  feedback.textContent = interceptToggle.checked
    ? "Interception enabled"
    : "Interception paused";
});

bypassBtn.addEventListener("click", async () => {
  await chrome.storage.local.set({ bypassOnce: true });
  feedback.textContent = "Next download will use browser";
});

loadState();
