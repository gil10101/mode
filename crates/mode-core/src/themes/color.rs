//! Color arithmetic for the theme validator and resolver (10 §3.2, §4.3,
//! §5.10, §11.1): `#rrggbb` / `#rrggbbaa` parsing, WCAG 2.1 relative
//! luminance and contrast ratio, OKLab / OKLCH conversions (the generated
//! avatar hues, the accent adjustment) and `color-mix(in oklab, …)`.

/// An sRGB color with straight alpha, components 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// Parse `#rrggbb` or `#rrggbbaa` (10 §3.1 "Units"); anything else is `None`.
pub fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.strip_prefix('#')?;
    if !(h.len() == 6 || h.len() == 8) || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16)
            .ok()
            .map(|v| v as f64 / 255.0)
    };
    Some(Rgba {
        r: byte(0)?,
        g: byte(2)?,
        b: byte(4)?,
        a: if h.len() == 8 { byte(6)? } else { 1.0 },
    })
}

/// Whether `s` is a valid color token value.
pub fn is_hex(s: &str) -> bool {
    parse_hex(s).is_some()
}

fn to_byte(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `#RRGGBB` (alpha dropped), upper case.
pub fn to_hex(c: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        to_byte(c.r),
        to_byte(c.g),
        to_byte(c.b)
    )
}

/// `#RRGGBBAA`.
pub fn to_hex_alpha(c: Rgba, alpha: f64) -> String {
    format!("{}{:02X}", to_hex(c), to_byte(alpha))
}

/// `fg` composited over an opaque `bg`.
pub fn over(fg: Rgba, bg: Rgba) -> Rgba {
    let a = fg.a;
    Rgba {
        r: fg.r * a + bg.r * (1.0 - a),
        g: fg.g * a + bg.g * (1.0 - a),
        b: fg.b * a + bg.b * (1.0 - a),
        a: 1.0,
    }
}

fn lin(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn gamma(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// WCAG 2.1 relative luminance.
pub fn luminance(c: Rgba) -> f64 {
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio of a text color on a background (alpha composited).
pub fn contrast(fg: Rgba, bg: Rgba) -> f64 {
    let bg = Rgba { a: 1.0, ..bg };
    let fg = over(fg, bg);
    let (a, b) = (luminance(fg), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Contrast of two hex strings; `None` when either does not parse.
pub fn contrast_hex(fg: &str, bg: &str) -> Option<f64> {
    Some(contrast(parse_hex(fg)?, parse_hex(bg)?))
}

/// OKLab `(L, a, b)`.
pub fn to_oklab(c: Rgba) -> (f64, f64, f64) {
    let (r, g, b) = (lin(c.r), lin(c.g), lin(c.b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    )
}

/// sRGB from OKLab, clamped to the gamut.
pub fn from_oklab(lab: (f64, f64, f64)) -> Rgba {
    let (ll, a, b) = lab;
    let l = (ll + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (ll - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (ll - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    let r = 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s;
    let g = -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s;
    let bl = -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s;
    Rgba {
        r: gamma(r.clamp(0.0, 1.0)),
        g: gamma(g.clamp(0.0, 1.0)),
        b: gamma(bl.clamp(0.0, 1.0)),
        a: 1.0,
    }
}

/// sRGB from OKLCH (`h` in degrees).
pub fn from_oklch(l: f64, c: f64, h: f64) -> Rgba {
    let rad = h.to_radians();
    from_oklab((l, c * rad.cos(), c * rad.sin()))
}

/// OKLCH `(L, C, h°)`.
pub fn to_oklch(c: Rgba) -> (f64, f64, f64) {
    let (l, a, b) = to_oklab(c);
    let h = b.atan2(a).to_degrees();
    (
        l,
        (a * a + b * b).sqrt(),
        if h < 0.0 { h + 360.0 } else { h },
    )
}

/// `color-mix(in oklab, <a> <pct>%, <b>)`.
pub fn mix_oklab(a: Rgba, pct: f64, b: Rgba) -> Rgba {
    let (x, y) = (to_oklab(a), to_oklab(b));
    let t = pct.clamp(0.0, 1.0);
    from_oklab((
        x.0 * t + y.0 * (1.0 - t),
        x.1 * t + y.1 * (1.0 - t),
        x.2 * t + y.2 * (1.0 - t),
    ))
}

/// The twelve generated avatar hues of 10 §5.10: 30° steps in OKLCH at
/// L 0.58 C 0.14 (light) or L 0.72 C 0.13 (dark), starting at red (29°).
pub fn generated_avatars(dark: bool) -> Vec<String> {
    let (l, c) = if dark { (0.72, 0.13) } else { (0.58, 0.14) };
    (0..12)
        .map(|i| to_hex(from_oklch(l, c, 29.0 + 30.0 * i as f64)))
        .collect()
}

/// Move `c`'s OKLCH lightness step by step (darker when `darken`) until
/// `ok(candidate)` holds; `None` when no lightness in range satisfies it.
pub fn adjust_lightness(c: Rgba, darken: bool, ok: impl Fn(Rgba) -> bool) -> Option<Rgba> {
    let (l, ch, h) = to_oklch(c);
    let mut cur = l;
    for _ in 0..100 {
        cur += if darken { -0.01 } else { 0.01 };
        if !(0.0..=1.0).contains(&cur) {
            return None;
        }
        let cand = from_oklch(cur, ch, h);
        if ok(cand) {
            return Some(cand);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_syntax() {
        let c = parse_hex("#D9480F").unwrap();
        assert_eq!(to_hex(c), "#D9480F");
        assert!(parse_hex("#d9480f80").is_some());
        assert!(
            parse_hex("#abc").is_none(),
            "three-digit form is not a token value"
        );
        assert!(parse_hex("D9480F").is_none());
        assert!(parse_hex("#GG480F").is_none());
    }

    #[test]
    fn contrast_matches_known_pairs() {
        // White on black is 21:1; Paper's white on the accent ≈ 4.3:1 (10 §3.2).
        assert!((contrast_hex("#FFFFFF", "#000000").unwrap() - 21.0).abs() < 0.01);
        let paper = contrast_hex("#FFFFFF", "#D9480F").unwrap();
        assert!((4.2..4.4).contains(&paper), "{paper}");
        // Paper's fg-3 on surface ≈ 3.5:1 (10 §3.2).
        let fg3 = contrast_hex("#8A877E", "#FAF9F6").unwrap();
        assert!((3.3..3.7).contains(&fg3), "{fg3}");
    }

    #[test]
    fn oklab_round_trip() {
        for hex in ["#D9480F", "#35669A", "#1E1D1A", "#FAF9F6", "#A6E3A1"] {
            let c = parse_hex(hex).unwrap();
            assert_eq!(to_hex(from_oklab(to_oklab(c))), hex);
        }
        assert_eq!(generated_avatars(false).len(), 12);
        assert!(generated_avatars(true).iter().all(|h| is_hex(h)));
    }
}
