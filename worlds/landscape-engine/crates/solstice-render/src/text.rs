//! What the renderer's text files (weather, sea states, particle effects) share in reading them.

/// A finite number, or why not.
pub(crate) fn number(word: &str) -> Result<f32, String> {
    word.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{word:?} is not a number"))
}
