//! Glass styling. Sizes scale with the keyboard unit so labels stay readable
//! at every overlay size; colours are fixed for legibility over any windows.
//!
//! Keys, panels and their controls share one set of theme values so the
//! keyboard and the menus read as one application.

/// Smoky surface behind panels, keys and tooltips (RGB).
const SURFACE: &str = "16, 20, 26";
/// Neutral hairline used for control and key borders.
const LINE: &str = "rgba(255,255,255,0.16)";
/// Raised control fill on top of a surface, and its hover state.
const RAISE: &str = "rgba(255,255,255,0.07)";
const RAISE_HOVER: &str = "rgba(255,255,255,0.13)";
const INK: &str = "rgba(236,241,245,0.97)";
/// Mint accent: chosen rows, active tabs, primary buttons, selected keys and
/// the "assigned" outline.
const ACCENT: &str = "110,231,183";
/// Red for errors and keys that make a selection unusable.
const DANGER: &str = "248,113,113";
/// Keyboard focus ring, deliberately neutral so it never reads as a status.
const FOCUS: &str = "rgba(236,241,245,0.85)";

/// Key legend font size, horizontal key padding and key border width, in
/// pixels, for a keyboard unit of `unit` pixels.
pub fn key_text_metrics(unit: f64) -> (f64, f64, f64) {
    let legend = (unit * 0.30).clamp(9.0, 17.0);
    let pad_h = (unit * 0.08).clamp(1.0, 6.0);
    let bw = ((unit * 0.032).clamp(1.0, 2.0) * 2.0).round() / 2.0;
    (legend, pad_h, bw)
}

/// `blurred`: the compositor blurs what is behind the overlay. Without blur,
/// the dim has a floor and key glass is more opaque so legends stay readable.
pub fn css(unit: f64, dim: f64, blurred: bool, reduced_motion: bool) -> String {
    let dim = if blurred { dim } else { dim.max(0.55) };
    let (legend, pad_h, bw) = key_text_metrics(unit);
    let pad_v = (unit * 0.04).clamp(0.0, 3.0);
    let small = (legend * 0.78).max(9.0);
    // Menu controls use 7 px corners at about 30 px height; keys keep that
    // proportion instead of copying the absolute value.
    let radius = (unit * 0.16).clamp(4.0, 9.0);
    // One border width for every key state (key_text_metrics), so status never changes key size.
    let surface = if blurred { 0.58 } else { 0.86 };
    let transition =
        if reduced_motion { "none" } else { "background-color 110ms ease-out, border-color 110ms ease-out, opacity 140ms ease-out" };
    format!(
        r#"
window.keymap-overlay {{ background: transparent; }}
window.keymap-overlay.km-warming {{ opacity: 0.004; }}
.km-backdrop {{ background-color: rgba(6, 9, 13, {dim:.3}); }}
.km-scrim {{ background-color: rgba(6, 9, 13, 0.18); }}

button.km-key {{
  padding: {pad_v:.1}px {pad_h:.1}px;
  min-width: 0;
  min-height: 0;
  outline: none;
}}
.km-key {{
  background-color: rgba({SURFACE}, {surface:.2});
  background-image: linear-gradient(180deg, {RAISE}, rgba(255,255,255,0.03));
  border: {bw:.1}px solid {LINE};
  border-radius: {radius:.1}px;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.06), 0 1px 2px rgba(0,0,0,0.30);
  color: {INK};
  transition: {transition};
}}
.km-key:hover {{ background-image: linear-gradient(180deg, {RAISE_HOVER}, rgba(255,255,255,0.06)); }}
.km-key:active {{ background-image: linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0.08)); }}
.km-key:focus-visible {{ outline: 2px solid {FOCUS}; outline-offset: 2px; }}
/* Selection is a solid fill with dark ink (like primary buttons), so it reads
   over bright windows too; status is the outline. Both stay visible together. */
.km-key.selected {{
  background-color: rgb({ACCENT});
  background-image: linear-gradient(180deg, rgba(255,255,255,0.14), rgba(255,255,255,0));
  color: rgb(9,24,19);
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.35), 0 1px 3px rgba(0,0,0,0.40);
}}
.km-key.selected:hover {{ background-image: linear-gradient(180deg, rgba(255,255,255,0.24), rgba(255,255,255,0.06)); }}
.km-key.assigned {{ border-color: rgb({ACCENT}); box-shadow: inset 0 0 0 1px rgba({ACCENT},0.45), 0 1px 2px rgba(0,0,0,0.30); }}
/* A mint outline vanishes on the mint fill: a dark inner ring marks it instead. */
.km-key.selected.assigned {{ border-color: rgb(9,24,19); box-shadow: inset 0 0 0 1px rgba(9,24,19,0.55), 0 1px 3px rgba(0,0,0,0.40); }}
.km-key.invalid {{ border-color: rgba({DANGER}, 0.95); }}
.km-key.faded {{ opacity: 0.22; }}
.km-key.disabled {{ opacity: 0.45; }}
.km-legend {{ font-size: {legend:.1}px; font-weight: 500; }}
.km-shifted {{ font-size: {small:.1}px; opacity: 0.72; }}

.km-hint, .km-tip, .km-panel, .km-toast {{
  background-color: rgba({SURFACE}, 0.84);
  border: 1px solid rgba(255,255,255,0.14);
  border-top-color: rgba(255,255,255,0.22);
  border-radius: 12px;
  box-shadow: 0 18px 48px rgba(0,0,0,0.45);
  color: {INK};
}}
.km-hint {{ padding: 8px 14px; border-radius: 9px; }}
.km-hint.error {{ border-color: rgba({DANGER},0.6); }}
.km-tip {{ padding: 10px 12px; border-radius: 9px; }}
.km-tip-title {{ font-weight: 600; }}
.km-tip-row {{ font-size: 13px; }}
.km-muted {{ color: rgba(220,228,235,0.62); }}
.km-small {{ font-size: 12px; }}
.km-error-text {{ color: rgb(252,165,165); }}
.km-mint {{ color: rgb(134,239,199); }}
.km-demo {{ background-color: rgba(250,204,21,0.16); border: 1px solid rgba(250,204,21,0.55); color: rgb(254,240,138); border-radius: 99px; padding: 3px 12px; font-size: 12px; }}

.km-panel {{ padding: 20px 22px; }}
.km-title {{ font-size: 20px; font-weight: 600; }}
.km-section {{ font-weight: 600; font-size: 13px; color: rgba(220,228,235,0.75); margin-top: 6px; }}
.km-cap {{
  background-image: linear-gradient(180deg, {RAISE_HOVER}, rgba(255,255,255,0.04));
  border: 1px solid {LINE};
  border-radius: 5px; padding: 2px 8px; font-size: 12px; min-height: 18px;
}}
.km-plus {{ color: rgba(220,228,235,0.55); font-size: 12px; }}

.km-panel button, .km-panel entry, .km-panel spinbutton, .km-panel dropdown > button, .km-panel textview {{
  background-image: none;
  background-color: {RAISE};
  border: 1px solid {LINE};
  border-radius: 7px;
  color: inherit;
  box-shadow: none;
}}
.km-panel button {{ padding: 5px 14px; }}
.km-panel button:hover {{ background-color: {RAISE_HOVER}; }}
.km-panel button:active {{ background-color: rgba(255,255,255,0.04); }}
.km-panel button:focus-visible, .km-panel entry:focus-within {{ outline: 2px solid {FOCUS}; outline-offset: 0; }}
.km-panel button:disabled {{ opacity: 0.45; }}
/* Choices in confirmations (which shortcut to replace). */
/* Confirmations: even padding, glass dense enough that keys behind never
   cross the text, one button height. */
.km-panel.km-dialog {{ padding: 24px; background-color: rgba({SURFACE}, 0.95); }}
/* Relative sizes, so the title stays prominent and keycaps stay readable
   under the desktop's text scaling (equal to 18px / 12px at the default). */
.km-dialog .km-title {{ font-size: 1.25em; }}
.km-dialog .km-cap, .km-dialog .km-plus, .km-dialog .km-small {{ font-size: 0.82em; }}
.km-dialog-action {{ font-weight: 600; }}
.km-dialog button {{ padding: 7px 16px; min-height: 22px; }}
.km-panel checkbutton {{ padding: 2px 6px; border-radius: 8px; outline: none; }}
.km-panel checkbutton:focus-visible {{ outline: 2px solid {FOCUS}; outline-offset: 0; }}
.km-panel switch:focus-visible {{ outline: 2px solid {FOCUS}; outline-offset: 2px; }}
.km-panel checkbutton:disabled {{ opacity: 0.45; }}
.km-panel checkbutton radio {{ background: rgba(255,255,255,0.06); border: 1px solid rgba(255,255,255,0.28); color: transparent; }}
.km-panel checkbutton radio:checked {{ background: rgb({ACCENT}); border-color: rgb({ACCENT}); color: rgb(9,24,19); }}
.km-panel button.km-primary {{ background-color: rgb({ACCENT}); color: rgb(9,24,19); border-color: rgb({ACCENT}); font-weight: 600; }}
.km-panel button.km-primary:hover {{ background-color: rgb(134,239,199); }}
.km-panel button.km-primary:disabled {{ background-color: rgba({ACCENT},0.25); color: rgba(236,241,245,0.6); border-color: transparent; }}
.km-panel button.km-danger {{ border-color: rgba({DANGER},0.6); color: rgb(252,165,165); }}
.km-panel button.km-tab {{ padding: 7px 14px; }}
.km-panel button.km-tab:checked {{ background-color: rgba({ACCENT},0.16); border-color: rgba({ACCENT},0.75); }}
.km-panel button.flat {{ background-color: transparent; border-color: transparent; padding: 2px 8px; }}
.km-panel button.flat:hover {{ background-color: rgba(255,255,255,0.09); }}

.km-list {{ background: transparent; }}
.km-list > row {{
  background-color: rgba(255,255,255,0.035);
  border: 1px solid rgba(255,255,255,0.08);
  border-radius: 8px; margin-bottom: 4px; padding: 6px 10px;
  transition: {transition};
}}
.km-list > row.km-header-row {{ background: transparent; border-color: transparent; padding: 0; }}
.km-list > row:hover {{ background-color: rgba(255,255,255,0.07); }}
.km-list > row:focus-visible {{ outline: 2px solid {FOCUS}; outline-offset: -2px; }}
.km-list > row.km-chosen {{ background-color: rgba({ACCENT},0.13); border-color: rgba({ACCENT},0.8); }}
/* The kept action list is a list view: the row is a bare container (focus,
   activation) and its .km-item child carries the card styling. */
listview.km-list > row {{ background: none; border: none; padding: 0; margin: 0; outline: none; }}
listview.km-list > header {{ background: none; padding: 0; }}
.km-item {{
  background-color: rgba(255,255,255,0.035);
  border: 1px solid rgba(255,255,255,0.08);
  border-radius: 8px; margin-bottom: 4px; padding: 6px 10px;
  transition: {transition};
}}
listview.km-list > row:hover .km-item {{ background-color: rgba(255,255,255,0.07); }}
listview.km-list > row:focus-visible .km-item {{ outline: 2px solid {FOCUS}; outline-offset: -2px; }}
.km-item.km-chosen {{ background-color: rgba({ACCENT},0.13); border-color: rgba({ACCENT},0.8); }}
.km-row-name {{ font-weight: 500; }}
.km-details {{ background-color: rgba(0,0,0,0.22); border-radius: 6px; padding: 8px 10px; margin-top: 6px; }}
.km-banner {{ background-color: rgba(250,204,21,0.10); border: 1px solid rgba(250,204,21,0.40); border-radius: 8px; padding: 6px 10px; }}
.km-banner.error {{ background-color: rgba({DANGER},0.10); border-color: rgba({DANGER},0.55); }}
.km-mono {{ font-family: monospace; font-size: 12px; }}
.km-toast {{ padding: 8px 10px 8px 16px; border-radius: 10px; }}
/* Scrollable panel areas: a fixed gutter beside the content (never over
   it), with a gap between rows and the track. See widgets::scroller. */
.km-scroll > scrollbar.vertical {{
  background: transparent; border: none;
  margin-left: 6px; padding: 0 2px;
}}
.km-scroll > scrollbar.vertical trough {{ background-color: rgba(255,255,255,0.05); border-radius: 99px; min-width: 6px; }}
.km-scroll > scrollbar.vertical slider {{
  background-color: rgba(255,255,255,0.30); border: none; border-radius: 99px;
  min-width: 6px; min-height: 28px; margin: 0;
}}
.km-scroll > scrollbar.vertical slider:hover {{ background-color: rgba(255,255,255,0.45); }}
.km-scroll > scrollbar.vertical slider:active {{ background-color: rgba({ACCENT},0.75); }}
.km-scroll.km-fits > scrollbar.vertical {{ opacity: 0; }}
scale trough {{ background-color: rgba(255,255,255,0.12); }}
scale highlight {{ background-color: rgb({ACCENT}); }}
switch {{ background-color: rgba(255,255,255,0.14); }}
switch:checked {{ background-color: rgb({ACCENT}); }}
"#
    )
}
