# TILIT für Windows

Rechte Strg-Taste halten, sprechen, loslassen: Der Text erscheint im aktiven Feld. Rust + Tauri 2, Plan und Entscheidungen in `docs/W0-01-windows-port.md`.

## Starten

```powershell
cargo build --release -p tilit
.\target\release\tilit.exe
```

TILIT läuft im Tray weiter, wenn du das Fenster schließt. Beenden geht über das Tray-Menü. Beim ersten Start trägst du unter **API-Keys** mindestens einen Key ein.

## Prüfen

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Aufbau

| Pfad | Inhalt |
|---|---|
| `core/` | Plattformneutrale Logik (`tilit-core`): Hotkey-Automat, Sitzung, Deepgram, Groq, Resampler, Ablage. Tests laufen ohne Mikrofon und Netzwerk. |
| `app/src/` | Windows-Anbindung: `hotkey`, `capture`, `insertion`, `secrets`, `hud`, `tray`, `commands`. |
| `app/ui/` | Einstellungsfenster und HUD, reines HTML/JS ohne Build-Schritt. |

## Wo Daten liegen

| Was | Wo |
|---|---|
| API-Keys | Windows-Anmeldeinformationsverwaltung, Dienst `de.gawinnovate.tilit` |
| Einstellungen, Verlauf | `%APPDATA%\TILIT` |
| Log (ohne Audio, Text und Keys) | `%LOCALAPPDATA%\de.gawinnovate.tilit\logs` |

## Bekannte Grenzen

- Fenster, die als Administrator laufen, sehen weder Hotkey noch Einfügen, solange TILIT nicht ebenfalls als Administrator läuft.
- Rechte Strg + Mausklick oder Mausrad zählt nicht als Tastenkombination: Nach 150 ms startet trotzdem eine Aufnahme.
- Keine `.env`-Datei: Keys kommen nur über das Fenster in den Schlüsselspeicher.
