let lastAltState = false;

function reportAlt(pressed) {
  if (pressed === lastAltState) return;
  lastAltState = pressed;
  try {
    chrome.runtime.sendMessage({ type: "alt_state", pressed });
  } catch {
    // Service worker may be waking up
  }
}

window.addEventListener("keydown", (e) => reportAlt(e.altKey), true);
window.addEventListener("keyup", (e) => reportAlt(e.altKey), true);
window.addEventListener("mousedown", (e) => reportAlt(e.altKey), true);
window.addEventListener("blur", () => reportAlt(false), true);
