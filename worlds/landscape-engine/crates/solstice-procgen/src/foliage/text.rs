//! Helpers for the text forms of foliage descriptions: one setting per line, a keyword first, then
//! numbers, `a..b` ranges, `r,g,b` colours or `key=value` pairs, with `#` starting a comment.

/// A finite number.
pub fn number(text: &str) -> Result<f32, String> {
    let value: f32 = text
        .parse()
        .map_err(|_| format!("{text:?} is not a number"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{text:?} is not a finite number"))
    }
}

/// `a..b`, or `a` for both ends.
pub fn range(text: &str) -> Result<(f32, f32), String> {
    match text.split_once("..") {
        Some((a, b)) => Ok((number(a)?, number(b)?)),
        None => number(text).map(|x| (x, x)),
    }
}

/// Writes a range as [`range`] reads it.
pub fn range_text((a, b): (f32, f32)) -> String {
    if a == b {
        a.to_string()
    } else {
        format!("{a}..{b}")
    }
}

/// The only word on a line after its keyword, as a number.
pub fn single(words: &[&str]) -> Result<f32, String> {
    match words {
        [value] => number(value),
        _ => Err("expected one number".into()),
    }
}

/// `key=value` words, split.
pub fn pairs<'a>(words: &[&'a str]) -> Result<Vec<(&'a str, &'a str)>, String> {
    words
        .iter()
        .map(|word| {
            word.split_once('=')
                .ok_or_else(|| format!("expected key=value, found {word:?}"))
        })
        .collect()
}

/// A colour written `r,g,b`, in linear albedo.
pub fn color(text: &str) -> Result<glam::Vec3, String> {
    let values: Vec<f32> = text.split(',').map(number).collect::<Result<_, _>>()?;
    match values[..] {
        [r, g, b] => Ok(glam::Vec3::new(r, g, b)),
        _ => Err(format!("{text:?} is not a colour: expected r,g,b")),
    }
}

/// Writes a colour as [`color`] reads it.
pub fn color_text(c: glam::Vec3) -> String {
    format!("{},{},{}", c.x, c.y, c.z)
}

/// Splits text into lines of words, without comments or blank lines, numbered from 1.
pub fn lines(text: &str) -> impl Iterator<Item = (usize, Vec<&str>)> {
    text.lines().enumerate().filter_map(|(index, raw)| {
        let words: Vec<&str> = raw
            .split('#')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        (!words.is_empty()).then_some((index + 1, words))
    })
}
