const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const TABS = [
  ["general", "Allgemein"], ["keys", "API-Keys"], ["models", "Modelle"],
  ["dictionary", "Wörterbuch"], ["replacements", "Ersetzungen"], ["history", "Verlauf"],
];
const CONSOLES = { deepgramNova3: "deepgramConsole", groqWhisper: "groqConsole" };
const VIEWS = { general, keys, models, dictionary, replacements, history };

let tab = "general";
let state = null;
// Rückmeldung der Key-Prüfung je Anbieter; überlebt das Neuzeichnen.
const keyMessages = {};

/** Baut ein Element. Texte gehen immer über textContent, nie als HTML. */
function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(props)) {
    if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
    else if (name === "class") node.className = value;
    else node[name] = value;
  }
  node.append(...children.filter((child) => child !== null && child !== undefined));
  return node;
}

const section = (title, ...children) => el("section", {}, title ? el("h3", {}, title) : null, ...children);
const row = (...children) => el("div", { class: "row" }, ...children);
const grow = (...children) => el("div", { class: "grow" }, ...children);
const patch = (change) => invoke("update_settings", { patch: change });

function statusRow(title, ok, detail, action) {
  return row(
    el("span", { class: ok ? "ok" : "warn" }, ok ? "✓" : "!"),
    grow(el("div", {}, title), ok ? null : el("div", { class: "muted small" }, detail)),
    ok ? null : action,
  );
}

function general() {
  const { settings } = state;
  const activeModel = state.models.find((model) => model.id === settings.speechModel);
  const language = el("select", { onchange: (event) => patch({ language: event.target.value }) },
    ...state.languages.map(({ id, title }) => el("option", { value: id, selected: id === settings.language }, title)));
  return [
    el("h1", {}, "Allgemein"),
    section(null,
      el("h2", {}, "Rechte Strg-Taste halten, sprechen, loslassen"),
      el("div", { class: "muted" }, "Der Text erscheint direkt im Feld, in dem du gerade schreibst.")),
    section("Bereitschaft",
      statusRow("Hotkey", state.hotkeyOk, "Der Tastenabgriff konnte nicht aktiviert werden. Starte TILIT neu."),
      statusRow("Mikrofon", state.microphoneOk, "In den Windows-Datenschutzeinstellungen gesperrt.",
        el("button", { onclick: () => invoke("open_target", { target: "microphoneSettings" }) }, "Öffnen…")),
      statusRow("API-Key", activeModel.hasKey, `Für ${activeModel.provider} fehlt noch ein Key.`,
        el("button", { onclick: () => select("keys") }, "Eintragen…"))),
    section("Diktat",
      row(grow("Sprache"), language),
      row(grow("Live einfügen, schon während du sprichst"),
        el("input", {
          type: "checkbox", checked: settings.liveMode, disabled: !activeModel.live,
          onchange: (event) => patch({ liveMode: event.target.checked }),
        })),
      row(grow("Diktate im Verlauf speichern"),
        el("input", {
          type: "checkbox", checked: settings.historyEnabled,
          onchange: (event) => patch({ historyEnabled: event.target.checked }),
        }))),
  ];
}

function keyMessageText(result, saved) {
  if (result.kind === "valid") return [saved ? "Gespeichert, Key ist gültig." : "Key ist gültig.", "ok"];
  if (result.kind === "invalid") {
    return [saved ? "Gespeichert, aber der Anbieter lehnt den Key ab." : "Der Anbieter lehnt den Key ab.", "bad"];
  }
  return [`${saved ? "Gespeichert. " : ""}Prüfung nicht möglich: ${result.reason}`, "bad"];
}

function keySection(model) {
  const input = el("input", { type: "password", placeholder: "Key einfügen", autocomplete: "off" });
  const report = (text, kind) => {
    keyMessages[model.id] = [text, kind];
    render();
  };
  const run = async (command, args, saved) => {
    report("Wird geprüft…", "muted");
    try {
      report(...keyMessageText(await invoke(command, args), saved));
    } catch (message) {
      report(String(message), "bad");
    }
  };
  const save = () => {
    if (input.value.trim()) run("save_key", { model: model.id, key: input.value }, true);
  };
  const verify = () => run("verify_key", { model: model.id, key: input.value }, false);
  const remove = async () => {
    delete keyMessages[model.id];
    await invoke("delete_key", { model: model.id });
  };
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") save();
  });
  const message = keyMessages[model.id];
  return section(`${model.provider} · ${model.title}`,
    row(el("span", { class: model.hasKey ? "ok" : "warn" }, model.hasKey ? "✓" : "!"),
      grow(model.hasKey ? "In der Windows-Anmeldeinformationsverwaltung gespeichert" : "Noch kein Key")),
    row(grow(input)),
    row(
      el("button", { onclick: save }, "Speichern"),
      el("button", { onclick: verify }, "Prüfen"),
      model.hasKey ? el("button", { class: "danger", onclick: remove }, "Entfernen") : null,
      grow(),
      el("button", { class: "link", onclick: () => invoke("open_target", { target: CONSOLES[model.id] }) }, "Key erstellen")),
    message ? el("div", { class: `small ${message[1]}` }, message[0]) : null);
}

function keys() {
  return [
    el("h1", {}, "API-Keys"),
    section(null, el("div", { class: "muted" },
      "Du brauchst mindestens einen Key. Er bleibt auf deinem PC in der Windows-Anmeldeinformationsverwaltung und wird nur an den jeweiligen Anbieter gesendet.")),
    ...state.models.map(keySection),
  ];
}

function models() {
  const choices = state.models.map((model) => {
    const selected = model.id === state.settings.speechModel;
    return el("button", {
      class: `model ${selected ? "selected" : ""}`,
      onclick: () => patch({ speechModel: model.id }),
    }, el("span", {}, selected ? "●" : "○"), el("span", {},
      el("div", {}, model.title, model.live ? el("span", { class: "tag" }, "Live") : null),
      el("div", { class: "muted small" }, model.detail),
      model.hasKey ? null : el("div", { class: "warn small" }, `Zugang für ${model.provider} fehlt`)));
  });
  return [
    el("h1", {}, "Modelle"),
    section("Spracherkennung", ...choices),
    section("Weitere Modelle",
      row(grow("Textbereinigung (Claude)"), el("span", { class: "muted" }, "Geplant")),
      row(grow("Lokal (ohne Internet)"), el("span", { class: "muted" }, "Geplant"))),
  ];
}

function dictionary() {
  const terms = state.settings.dictionary;
  const input = el("input", { type: "text", placeholder: "Neuer Begriff, z. B. Claude Code" });
  const add = () => {
    const term = input.value.trim();
    input.value = "";
    if (term && !terms.includes(term)) patch({ dictionary: [term, ...terms] });
  };
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") add();
  });
  const remove = (term) => patch({ dictionary: terms.filter((item) => item !== term) });
  return [
    el("h1", {}, "Wörterbuch"),
    section(null,
      el("div", { class: "muted" }, "Begriffe, die richtig geschrieben werden sollen: Namen, Produkte, Fachwörter. Sie gelten ab dem nächsten Diktat."),
      row(grow(input), el("button", { onclick: add }, "Hinzufügen"))),
    terms.length
      ? section(null, ...terms.map((term) => row(grow(term),
        el("button", { class: "danger", onclick: () => remove(term) }, "Entfernen"))))
      : el("div", { class: "empty" }, "Noch keine Begriffe"),
  ];
}

function replacements() {
  const rules = state.settings.replacements;
  const change = (index, field, value) => patch({
    replacements: rules.map((rule, position) => (position === index ? { ...rule, [field]: value } : rule)),
  });
  const add = () => patch({ replacements: [...rules, { id: crypto.randomUUID(), from: "", to: "" }] });
  const remove = (id) => patch({ replacements: rules.filter((rule) => rule.id !== id) });
  return [
    el("h1", {}, "Ersetzungen"),
    section(null,
      el("div", { class: "muted" }, "Wird im erkannten Text ersetzt, immer ganze Wörter und ohne Rücksicht auf Groß-/Kleinschreibung. Beispiel: „slash commit“ → „/commit“."),
      row(grow(), el("button", { onclick: add }, "Hinzufügen"))),
    rules.length
      ? section(null, ...rules.map((rule, index) => row(
        grow(el("input", { type: "text", placeholder: "Gesagt", value: rule.from, onchange: (event) => change(index, "from", event.target.value) })),
        el("span", { class: "muted" }, "→"),
        grow(el("input", { type: "text", placeholder: "Ersetzen durch", value: rule.to, onchange: (event) => change(index, "to", event.target.value) })),
        el("button", { class: "danger", onclick: () => remove(rule.id) }, "Entfernen"))))
      : el("div", { class: "empty" }, "Noch keine Ersetzungen"),
  ];
}

function history() {
  const entries = state.history;
  if (!entries.length) {
    const text = state.settings.historyEnabled ? "Noch keine Diktate" : "Verlauf ist ausgeschaltet (Allgemein)";
    return [el("h1", {}, "Verlauf"), el("div", { class: "empty" }, text)];
  }
  const words = entries.reduce((sum, entry) => sum + entry.text.split(/\s+/).filter(Boolean).length, 0);
  const clear = el("button", { class: "danger" }, "Verlauf löschen");
  clear.addEventListener("click", () => {
    // Zweiter Klick bestätigt: Browser-Dialoge sind in der App nicht sinnvoll.
    if (clear.dataset.armed) invoke("clear_history");
    else {
      clear.dataset.armed = "1";
      clear.textContent = "Wirklich alles löschen?";
    }
  });
  const format = new Intl.DateTimeFormat("de-DE", { dateStyle: "medium", timeStyle: "short" });
  return [
    el("h1", {}, "Verlauf"),
    row(el("div", { class: "stats grow" },
      el("span", {}, el("b", {}, String(entries.length)), "Diktate"),
      el("span", {}, el("b", {}, String(words)), "Wörter")), clear),
    section(null, ...entries.map((entry) => el("div", { class: "row entry" },
      grow(el("div", { class: "muted small" }, format.format(new Date(entry.date))), el("p", {}, entry.text)),
      el("button", { onclick: () => invoke("copy_to_clipboard", { text: entry.text }) }, "Kopieren"),
      el("button", { class: "danger", onclick: () => invoke("remove_history", { id: entry.id }) }, "Entfernen")))),
  ];
}

function select(next) {
  tab = next;
  render();
}

function render() {
  if (!state) return;
  document.getElementById("tabs").replaceChildren(...TABS.map(([id, title]) =>
    el("button", { class: id === tab ? "active" : "", onclick: () => select(id) }, title)));
  const badge = document.getElementById("badge");
  badge.className = state.phase;
  document.getElementById("badge-text").textContent = state.statusText;
  const page = document.getElementById("page");
  const top = page.scrollTop;
  page.replaceChildren(...VIEWS[tab]().filter(Boolean));
  page.scrollTop = top;
}

/** Nicht neu zeichnen, solange der Nutzer tippt: sonst wäre die Eingabe weg. */
function isTyping() {
  const active = document.activeElement;
  return active?.tagName === "INPUT" && active.type !== "checkbox";
}

async function refresh() {
  state = await invoke("snapshot");
  if (isTyping()) {
    const badge = document.getElementById("badge");
    badge.className = state.phase;
    document.getElementById("badge-text").textContent = state.statusText;
    return;
  }
  render();
}

listen("state-changed", refresh);
invoke("snapshot").then((first) => {
  state = first;
  if (first.models.every((model) => !model.hasKey)) tab = "keys";
  render();
});
