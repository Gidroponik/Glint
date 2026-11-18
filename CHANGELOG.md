# Changelog

## Unreleased

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
