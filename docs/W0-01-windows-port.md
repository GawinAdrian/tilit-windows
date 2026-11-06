# Feature: TILIT für Windows

> Ticket: W0-01 | Branch: n/a (Ordner ist kein Git-Repo) | Status: in-progress | Autonomy: supervised | Confidence: 7/10

## Feature Description

Windows-Port der macOS-App TILIT mit dem Funktionsumfang von `Sources/TILIT`: Taste halten, sprechen, loslassen, Text landet im aktiven Feld. Tray-App, Live-Text mit Deepgram, Groq Whisper, Aufnahme-HUD, API-Keys, Wörterbuch, Ersetzungen, Verlauf.

## User Story

Als Adrian will ich TILIT auf meinem Windows-PC benutzen, damit ich dort genauso diktieren kann wie am Mac.

## Problem Statement

Der Swift-Code hängt an AppKit, SwiftUI, AVFoundation, CGEventTap und Keychain und läuft nicht auf Windows. Windows war in PRD und ARCHITECTURE ein Nicht-Ziel; Adrian hat das am 2026-11-06 geändert.

## Entscheidungen von Adrian (2026-11-06)

- Stack: Rust + Tauri 2. Einstellungs-UI als Web-UI über WebView2, ohne Node-Build.
- Hotkey: rechte Strg-Taste halten (AltGr bleibt frei für @, €, {, [).
- Umfang: alles wie am Mac.

## Solution Statement

Neuer Ordner `windows/`, die Swift-App bleibt unverändert.

- `windows/core` (Crate `tilit-core`): plattformneutrale Logik, per `cargo test` headless testbar. Hardware und Anbieter stehen hinter Traits mit Fakes.
- `windows/app` (Crate `tilit`): dünne Tauri-App mit Win32-Anbindung.

| macOS | Windows |
|---|---|
| CGEventTap, rechte Option | `WH_KEYBOARD_LL`, rechte Strg |
| AVAudioEngine | cpal (WASAPI), eigener Resampler auf 16 kHz mono |
| NSPasteboard, Cmd+V | Win32-Zwischenablage, `SendInput` Strg+V |
| Keychain | Windows-Anmeldeinformationsverwaltung |
| NSPanel-HUD | Tauri-Fenster mit `WS_EX_NOACTIVATE` |
| NSStatusItem | Tauri-Tray |
| Application Support, UserDefaults | `%APPDATA%\TILIT` (JSON) |

## Out of Scope / Non-Goals

- Nicht enthalten: Claude-Textbereinigung und lokales Modell (am Mac auch nur "Geplant").
- Nicht enthalten: Installer, Code-Signing, Autostart.
- Nicht enthalten: `.env`-Rückfall für Keys. CLAUDE.md erlaubt Keys nur im Systemspeicher.
- Nicht geändert: alles unter `Sources/`, `Tests/`, `Package.swift`.

## Testing Strategy

Erwartungswerte stammen aus `Tests/TILITCoreTests` (Swift) und werden 1:1 übernommen. Hotkey-Tests werden sinngemäß auf Strg übertragen. Zusätzlich für Windows:

- Nach einem Abbruch startet Tastenwiederholung der noch gehaltenen rechten Strg-Taste kein neues Diktat.
- Sitzungsablauf gegen Fakes, Zeiten aus `DictationController.swift`: Haltezeit 150 ms, Nachlauf 250 ms, Mindestdauer 0,25 s, Maximum 300 s, Fehleranzeige 6 s, Live-Ende höchstens 3 s, Pause zwischen Abschnitten 150 ms.

## Validation

- `cargo test` und `cargo clippy -- -D warnings` in `windows/`
- `cargo build --release -p tilit`

## Manual QA

- [ ] Rechte Strg halten, sprechen, loslassen: Text erscheint in Editor, Browser und Slack.
- [ ] Vorherige Zwischenablage ist danach wiederhergestellt (Text und Bild).
- [ ] Antippen unter 150 ms bewirkt nichts; Strg+C mit rechter Strg kopiert normal.
- [ ] AltGr+Q ergibt weiter @.
- [ ] Fokuswechsel während des Diktats: Text landet in der Zwischenablage.
- [ ] Mikrofonwechsel (USB, Headset) funktioniert beim nächsten Diktat.
- [ ] HUD stiehlt nie den Fokus.
- [ ] Deepgram live und Groq liefern Text in de/en/pl.

## Bekannte Grenzen

Fenster, die als Administrator laufen, sehen weder Hotkey noch Einfügen, solange TILIT nicht ebenfalls als Administrator läuft (Windows UIPI).

## Amendments

none
