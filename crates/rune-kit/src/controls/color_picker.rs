//! ColorPicker: a color picked by its hue, saturation and value, as the pickers in design tools
//! and the web's `<input type="color">` dialogs pick them. A square of every saturation and value
//! of one hue, a strip of the hues, the color beside its hex code (typed in, when the app gives
//! it a field), and optional swatches to pick from.
//!
//! It works in [`Hsv`] rather than [`Color`] so that a gray, which has no hue of its own, keeps
//! the hue the square shows while it's dragged through: the app keeps the `Hsv` the picker hands
//! back and builds the picker from it again.

use std::rc::Rc;

use rune::{Color, Drag, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};

use crate::{Kit, TextInput};

/// A color as hue (0 to 1 around the wheel, red at 0), saturation and value (0 to 1), of its
/// sRGB encoding, as color pickers and CSS's `hsv()` describe it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Hsv {
    pub fn new(h: f32, s: f32, v: f32) -> Hsv {
        Hsv { h: h.rem_euclid(1.0), s: s.clamp(0.0, 1.0), v: v.clamp(0.0, 1.0) }
    }

    /// The hue, saturation and value of `color` (its alpha aside). A gray's hue is 0.
    pub fn of(color: Color) -> Hsv {
        Hsv::of_srgb([color.r, color.g, color.b].map(encode))
    }

    /// From sRGB-encoded channels, 0 to 1.
    fn of_srgb([r, g, b]: [f32; 3]) -> Hsv {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d <= 0.0 {
            0.0
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        Hsv::new(h, if max <= 0.0 { 0.0 } else { d / max }, max)
    }

    /// The opaque color.
    pub fn color(self) -> Color {
        let [r, g, b] = self.srgb();
        Color { r: decode(r), g: decode(g), b: decode(b), a: 1.0 }
    }

    /// `#rrggbb`, lowercase.
    pub fn hex(self) -> String {
        let [r, g, b] = self.srgb().map(|c| (c * 255.0).round() as u8);
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// `#rgb` or `#rrggbb` (the `#` optional), or None.
    pub fn from_hex(s: &str) -> Option<Hsv> {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        let digits: Vec<u8> = s.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
        let channel = |i: usize| match digits.len() {
            3 => Some(digits[i] * 17),
            6 => Some(digits[2 * i] * 16 + digits[2 * i + 1]),
            _ => None,
        };
        Some(Hsv::of_srgb([channel(0)?, channel(1)?, channel(2)?].map(|c| f32::from(c) / 255.0)))
    }

    fn srgb(self) -> [f32; 3] {
        let (h, s, v) = (self.h.rem_euclid(1.0) * 6.0, self.s, self.v);
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        [r + m, g + m, b + m]
    }
}

/// sRGB's transfer function, from linear light to its encoding and back.
fn encode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}
fn decode(c: f32) -> f32 {
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// How far an arrow key moves the square's point, and the hue strip's.
const STEP: f32 = 0.02;
const HUE_STEP: f32 = 1.0 / 72.0;
const THUMB: f32 = 16.0;
const STRIP: f32 = 12.0;
const SWATCH: f32 = 22.0;

type Change<S> = Rc<dyn Fn(&mut S, Hsv, &mut Tx)>;
type Toggle<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type Field<S> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut TextEdit>;

pub struct ColorPicker<S: 'static> {
    hsv: Hsv,
    label: Option<String>,
    width: f32,
    height: f32,
    swatches: Vec<Color>,
    change: Option<Change<S>>,
    hex: Option<(TextEdit, Field<S>)>,
    eyedropper: Option<(bool, Toggle<S>)>,
}

impl<S: 'static> ColorPicker<S> {
    pub fn new(hsv: Hsv) -> ColorPicker<S> {
        ColorPicker { hsv, label: None, width: 240.0, height: 150.0, swatches: Vec::new(), change: None, hex: None, eyedropper: None }
    }
    /// The accessible name of the color picked ("Background color").
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = Some(l.into());
        self
    }
    /// The picker's width (default 240) and its square's height (default 150), in points.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }
    /// Colors to pick with one press, in a row under the rest: ones that go with what the
    /// color is for, or the ones picked lately.
    pub fn swatches(mut self, colors: impl IntoIterator<Item = Color>) -> Self {
        self.swatches = colors.into_iter().collect();
        self
    }
    /// Called with the new color as the square or the strip is dragged or stepped, a swatch
    /// pressed, or a whole hex code typed. Keep the `Hsv` and build the picker from it.
    pub fn on_change(mut self, f: impl Fn(&mut S, Hsv, &mut Tx) + 'static) -> Self {
        self.change = Some(Rc::new(f));
        self
    }
    /// A field to type the hex code in, editing the app's `TextEdit` (`field` finds it in the
    /// state). A whole code typed changes the color; the picker keeps the field's text in step
    /// when the color changes any other way.
    pub fn hex_field(mut self, edit: &TextEdit, field: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static) -> Self {
        self.hex = Some((edit.clone(), Rc::new(field)));
        self
    }

    /// A button beside the hex code to pick a color from what the app shows: an image, a
    /// photo, a canvas. The app decides where picking happens; `active` is whether it's picking
    /// now, which the button shows pressed, and `toggle` is called when it's pressed. The app
    /// hands what it picks to the picker's `on_change` handler as any other change.
    pub fn eyedropper(mut self, active: bool, toggle: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.eyedropper = Some((active, Rc::new(toggle)));
        self
    }

    pub fn build(self, k: &Kit) -> El {
        let c = &k.c;
        let Hsv { h, s: sat, v } = self.hsv;
        let (w, area_h) = (self.width, self.height);
        let field = self.hex.as_ref().map(|(_, field)| field.clone());
        // Every change but typing the code puts the new code in the field
        let changed = self.change.clone().map(|f| -> Change<S> {
            let field = field.clone();
            Rc::new(move |s: &mut S, hsv: Hsv, tx: &mut Tx| {
                f(s, hsv, tx);
                if let Some(field) = &field {
                    field(s).replace_text(hsv.hex());
                }
            })
        });

        let pure = Hsv::new(h, 1.0, 1.0).color();
        let thumb = |key: &str, at: [f32; 2], color: Color| {
            el().key(key.to_owned())
                .absolute(at[0] - THUMB / 2.0, at[1] - THUMB / 2.0)
                .size(THUMB, THUMB)
                .radius(THUMB / 2.0)
                .fill(color)
                .border(2.0, Color::WHITE)
                .box_shadow(rune::Shadow { blur: 4.0, color: Color::BLACK.alpha(0.45), ..Default::default() })
                .a11y_hidden()
        };

        // The square: the hue, whitened to the left and darkened downwards
        let mut area = el()
            .key("area")
            .size(w, area_h)
            .radius(6.0)
            .fill(pure)
            .role("slider")
            .label("Saturation and brightness")
            .description(format!("{}% saturation, {}% brightness", (sat * 100.0).round(), (v * 100.0).round()))
            .focusable(true)
            .cursor(rune::Cursor::Crosshair)
            .child(
                el().absolute(0.0, 0.0)
                    .size(w, area_h)
                    .radius(6.0)
                    .linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, Color::WHITE), (1.0, Color::WHITE.alpha(0.0))]),
            )
            .child(el().absolute(0.0, 0.0).size(w, area_h).radius(6.0).gradient(Color::BLACK.alpha(0.0), Color::BLACK))
            .child(
                el().absolute(0.0, 0.0)
                    .size(w, area_h)
                    .radius(6.0)
                    .box_shadow(rune::Shadow { spread: 0.5, color: c.border_base, inset: true, ..Default::default() }),
            )
            .child(thumb("area-thumb", [sat * w, (1.0 - v) * area_h], self.hsv.color()));
        // The strip of hues, red round to red
        let hues: Vec<(f32, Color)> = (0..=6).map(|i| (i as f32 / 6.0, Hsv::new(i as f32 / 6.0, 1.0, 1.0).color())).collect();
        let mut strip = el()
            .key("hue")
            .size(w, STRIP)
            .radius(STRIP / 2.0)
            .linear_gradient(std::f32::consts::FRAC_PI_2, hues)
            .role("slider")
            .label("Hue")
            .range_value(f64::from((h * 360.0).round()), 0.0, 360.0)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .child(thumb("hue-thumb", [h * w, STRIP / 2.0], pure));
        if let Some(f) = &changed {
            let at = {
                let f = f.clone();
                move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                    f(s, Hsv::new(h, d.x / d.w.max(1.0), 1.0 - d.y / d.h.max(1.0)), tx)
                }
            };
            let keys = {
                let f = f.clone();
                move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
                    let (ds, dv) = match ev.key {
                        Key::Left => (-STEP, 0.0),
                        Key::Right => (STEP, 0.0),
                        Key::Up => (0.0, STEP),
                        Key::Down => (0.0, -STEP),
                        _ => return false,
                    };
                    f(s, Hsv::new(h, sat + ds, v + dv), tx);
                    true
                }
            };
            area = area.on_drag(at).on_key(keys);
            let hue = {
                let f = f.clone();
                move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                    // The strip's ends are both red: keep the hue just short of a turn at the right
                    f(s, Hsv::new((d.x / d.w.max(1.0)).clamp(0.0, 0.9999), sat, v), tx)
                }
            };
            let step = {
                let f = f.clone();
                move |s: &mut S, delta: i32, tx: &mut Tx| f(s, Hsv::new(h + delta as f32 * HUE_STEP, sat, v), tx)
            };
            strip = strip.on_drag(hue).on_step(step);
        }

        // The color, and its code to read or type
        let swatch = el()
            .key("swatch")
            .size(28.0, 28.0)
            .radius(6.0)
            .fill(self.hsv.color())
            .box_shadow(rune::Shadow { spread: 0.5, color: c.border_strong, ..Default::default() })
            .a11y_hidden();
        let code = match (&self.hex, &self.change) {
            (Some((edit, field)), Some(f)) => {
                let (field, f) = (field.clone(), f.clone());
                let read = field.clone();
                TextInput::new(edit)
                    .label("Hex color")
                    .placeholder("#rrggbb")
                    .width(None)
                    .on_edit(move |s: &mut S| field(s))
                    .on_change(move |s: &mut S, tx: &mut Tx| {
                        if let Some(hsv) = Hsv::from_hex(&read(s).text) {
                            f(s, hsv, tx);
                        }
                    })
                    .build(k)
                    .key("hex")
                    .grow(1.0)
                    .min_w(0.0)
            }
            _ => text(self.hsv.hex()).key("hex").font(k.ty.ui).color(c.text_muted).grow(1.0),
        };

        let dropper = self.eyedropper.map(|(active, toggle)| {
            let button = el()
                .key("eyedropper")
                .size(28.0, 28.0)
                .radius(6.0)
                .center()
                .fill(if active { c.bg_accent } else { c.layer_01 })
                .box_shadow(rune::Shadow { spread: 0.5, color: c.border_strong, ..Default::default() })
                .role("switch")
                .label("Pick a color")
                .checked(active)
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .child(rune::icon(&crate::glyphs::EYEDROPPER, 16.0).color(if active { c.white } else { c.icon_base }));
            k.interactive(button).on_press_tx(move |s: &mut S, tx: &mut Tx| toggle(s, tx))
        });
        let mut root = col().role("group").w(w).gap(12.0).child(area).child(strip).child(
            row().w_full().gap(10.0).center_y().child(swatch).child(code).children(dropper),
        );
        if let Some(l) = self.label {
            root = root.label(l);
        }
        if !self.swatches.is_empty() {
            let picked = self.hsv.hex();
            let chips = self.swatches.into_iter().enumerate().map(|(i, color)| {
                let hsv = Hsv::of(color);
                let on = hsv.hex() == picked;
                let mut chip = el()
                    .key(format!("swatch{i}"))
                    .size(SWATCH, SWATCH)
                    .radius(SWATCH / 2.0)
                    .fill(color.alpha(1.0))
                    .box_shadow(rune::Shadow { spread: if on { 2.0 } else { 0.5 }, color: if on { c.text_base } else { c.border_strong }, ..Default::default() })
                    .role("radio")
                    .label(hsv.hex())
                    .checked(on)
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer);
                if let Some(f) = &changed {
                    let f = f.clone();
                    chip = k.interactive(chip).on_press_tx(move |s: &mut S, tx: &mut Tx| f(s, hsv, tx));
                }
                chip
            });
            root = root.child(row().role("radiogroup").label("Swatches").focus_group().w_full().flex_wrap().gap(8.0).children(chips));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_go_round_hsv_and_hex() {
        for code in ["#ff0000", "#00ff80", "#123456", "#ffffff", "#000000", "#7f7f7f"] {
            assert_eq!(Hsv::from_hex(code).unwrap().hex(), code);
        }
        let red = Hsv::from_hex("f00").unwrap();
        assert_eq!((red.h, red.s, red.v), (0.0, 1.0, 1.0));
        let green = Hsv::from_hex("#00ff00").unwrap();
        assert!((green.h - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(Hsv::from_hex("#12345"), None);
        assert_eq!(Hsv::from_hex("nothex"), None);
    }

    #[test]
    fn a_gray_keeps_whatever_hue_it_is_given() {
        // Dragged to the square's left edge the color is gray, but the hue stays with the Hsv
        let gray = Hsv::new(0.6, 0.0, 0.5);
        assert_eq!(gray.h, 0.6);
        assert_eq!(gray.hex(), "#808080");
    }
}
