const pill = document.getElementById("pill");
const label = document.getElementById("text");
const MAX_INTERIM = 48;
let phase = "idle";
let phaseText = "";

window.__TAURI__.event.listen("hud", ({ payload }) => {
  phase = payload.phase;
  phaseText = payload.text;
  pill.className = phase;
  label.textContent = phaseText;
});

window.__TAURI__.event.listen("hud-interim", ({ payload }) => {
  if (phase !== "recording") return;
  const shown = payload.length > MAX_INTERIM ? "…" + payload.slice(-MAX_INTERIM) : payload;
  label.textContent = shown || phaseText;
});
