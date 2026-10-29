//! Ancho conservador de clusters locales. No presupone que exista cada glifo.
pub(super) fn scalar_width(c: char) -> usize {
    let n = u32::from(c);
    if c.is_control()
        || matches!(n,0x200c..=0x200d|0x300..=0x36f|0x1ab0..=0x1aff|0x1dc0..=0x1dff|0x20d0..=0x20ff|0xfe00..=0xfe0f|0xfe20..=0xfe2f|0xe0100..=0xe01ef)
    {
        0
    } else if matches!(n,0x1100..=0x115f|0x2329..=0x232a|0x2e80..=0xa4cf|0xac00..=0xd7a3|0xf900..=0xfaff|0xfe10..=0xfe19|0xfe30..=0xfe6f|0xff01..=0xff60|0xffe0..=0xffe6|0x1f000..=0x1faff|0x20000..=0x3ffff)
    {
        2
    } else {
        1
    }
}
pub(super) fn clusters(text: &str) -> impl Iterator<Item = (&str, usize)> {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        if cursor >= text.len() {
            return None;
        }
        let next = super::model::next_boundary(text, cursor);
        let cluster = &text[cursor..next];
        cursor = next;
        let width = if cluster.contains('\u{fe0f}') || cluster.contains('\u{20e3}') {
            2
        } else {
            cluster.chars().map(scalar_width).max().unwrap_or(0)
        };
        Some((cluster, width))
    })
}
