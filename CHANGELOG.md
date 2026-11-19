# Changelog

## 0.4.0 — 2026-11-19

### Added

- **All monitors at once.** The overlay now opens on every monitor, so a region can be selected on any of them no matter where the cursor was when the hotkey was pressed. The first click decides which monitor is being captured; the overlays on the others close.

### Fixed

- **Monitors with different scaling.** On a second monitor whose scale differs from the main one, the overlay could shrink and the copied region did not match the selection. The overlay now always fills the monitor it was opened on and follows that monitor's scale. The same fix applies to pinned captures and the settings window.

## 0.3.0 — 2026-11-19

### Added

- **Pin on top.** The pin button or `Ctrl+P` keeps the selection as a small window above all others, right where it was captured. Drag to move, pull a corner or use the mouse wheel to resize (proportions are kept), `Ctrl+C` to copy, double-click, right click or `Esc` to close.
- **QR code reader.** QR codes on the captured screen are found automatically: dark-on-light, light-on-dark and stylized ones with rounded dots and a logo in the middle. Each one is outlined as soon as the overlay opens and gets a label with its content; a click copies it. After a region is selected, only the codes inside it stay marked.
- **Spotlight tool.** Drag one or more rectangles and everything around them is dimmed. The thickness buttons set how dark the rest gets.

### Changed

- **Less memory in the tray.** A few seconds after the last Glint window closes, unused memory is handed back to the system. Idle use in Task Manager drops from tens of megabytes to a few.

## 0.2.0 — 2026-11-18

### Added

- **Blur tool.** Drag a rectangle to hide part of the capture. The thickness buttons set the strength. Blur always stays beneath other annotations.
- **Selectable text.** A text mode (`Ctrl+T` or `0`) recognizes the words in the selected area with the OCR engine built into Windows. Drag over words to select them, double-click for a line, triple-click or `Ctrl+A` for everything, then copy. With nothing selected, all recognized text is copied.
- **Magnifier.** While selecting a region, a loupe next to the cursor shows the surrounding pixels and the color under the cursor.
- **Color picker.** Before selecting a region, `C` or a right click copies the color under the cursor as `#RRGGBB`.
- **Window selection.** The window under the cursor is outlined; a click selects it.
- Tool shortcuts now cover all ten tools: `1`–`9` and `0`.

### Changed

- A right click with no region selected now copies the color under the cursor and closes the overlay. Use `Esc` to close without copying.

## 0.1.0

First release: region and full-screen capture, pen, marker, line, arrow, rectangle, ellipse and text, shape smoothing, PNG / PNG-8 / JPEG / WebP output, global hotkeys, tray icon, start with Windows.
