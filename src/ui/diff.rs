//! Vista de diff de líneas con comparación acotada y tonos propios, sin ANSI.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiffTone {
    Context,
    Added,
    Removed,
    Hunk,
    Notice,
}

pub(super) struct FormattedDiff {
    pub text: String,
    /// Índice de línea lógica, antes de envolverla según el ancho de consola.
    pub highlights: BTreeMap<usize, DiffTone>,
}

#[derive(Clone, Copy)]
struct Operation<'a> {
    tone: DiffTone,
    raw: &'a str,
}

#[derive(Default)]
struct Output {
    lines: Vec<String>,
    highlights: BTreeMap<usize, DiffTone>,
}

impl Output {
    fn push(&mut self, text: impl Into<String>, tone: DiffTone) {
        self.highlights.insert(self.lines.len(), tone);
        self.lines.push(text.into());
    }

    fn finish(self) -> FormattedDiff {
        FormattedDiff {
            text: self.lines.join("\n"),
            highlights: self.highlights,
        }
    }
}

pub(super) fn format(before: &str, after: &str, context: usize, max_cells: usize) -> FormattedDiff {
    let mut output = Output::default();
    output.push(
        format!("Saltos antes: {}", endings(before)),
        DiffTone::Notice,
    );
    output.push(
        format!("Saltos después: {}", endings(after)),
        DiffTone::Notice,
    );
    if before.starts_with('\u{feff}') || after.starts_with('\u{feff}') {
        let presence = |text: &str| {
            if text.starts_with('\u{feff}') {
                "sí"
            } else {
                "no"
            }
        };
        output.push(
            format!(
                "BOM UTF-8: antes {}; después {}",
                presence(before),
                presence(after)
            ),
            DiffTone::Notice,
        );
    }
    if visible_escapes_needed(before) || visible_escapes_needed(after) {
        output.push(
            "Representación: controles y BOM se muestran como escapes; el archivo no se modifica.",
            DiffTone::Notice,
        );
    }
    if before == after {
        output.push("Sin cambios de contenido.", DiffTone::Notice);
        return output.finish();
    }

    let old = raw_lines(before);
    let new = raw_lines(after);
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut operations = Vec::new();
    extend(&mut operations, &old[..prefix], DiffTone::Context);
    let grouped = middle(
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
        max_cells,
        &mut operations,
    );
    extend(
        &mut operations,
        &old[old.len() - suffix..],
        DiffTone::Context,
    );
    if grouped {
        output.push(
            "Diff agrupado: límite de comparación alcanzado; el bloque puede incluir coincidencias internas.",
            DiffTone::Notice,
        );
    }
    output.push("--- Antes", DiffTone::Notice);
    output.push("+++ Después", DiffTone::Notice);
    render_hunks(&mut output, &operations, context);
    output.finish()
}

fn raw_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split_inclusive('\n').collect()
    }
}

fn extend<'a>(operations: &mut Vec<Operation<'a>>, lines: &[&'a str], tone: DiffTone) {
    operations.extend(lines.iter().map(|raw| Operation { tone, raw }));
}

/// true indica agrupación por presupuesto/recursos, sin fingir un diff mínimo.
fn middle<'a>(
    old: &[&'a str],
    new: &[&'a str],
    max_cells: usize,
    operations: &mut Vec<Operation<'a>>,
) -> bool {
    if old.is_empty() || new.is_empty() {
        extend(operations, old, DiffTone::Removed);
        extend(operations, new, DiffTone::Added);
        return false;
    }
    let dimensions = old.len().checked_add(1).and_then(|rows| {
        new.len()
            .checked_add(1)
            .and_then(|columns| rows.checked_mul(columns).map(|cells| (columns, cells)))
    });
    let Some((columns, cells)) = dimensions.filter(|(_, cells)| *cells <= max_cells) else {
        extend(operations, old, DiffTone::Removed);
        extend(operations, new, DiffTone::Added);
        return true;
    };
    let mut lengths = Vec::<u32>::new();
    if lengths.try_reserve_exact(cells).is_err() {
        extend(operations, old, DiffTone::Removed);
        extend(operations, new, DiffTone::Added);
        return true;
    }
    lengths.resize(cells, 0);
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i * columns + j] = if old[i] == new[j] {
                lengths[(i + 1) * columns + j + 1] + 1
            } else {
                lengths[(i + 1) * columns + j].max(lengths[i * columns + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            operations.push(Operation {
                tone: DiffTone::Context,
                raw: old[i],
            });
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * columns + j] >= lengths[i * columns + j + 1] {
            operations.push(Operation {
                tone: DiffTone::Removed,
                raw: old[i],
            });
            i += 1;
        } else {
            operations.push(Operation {
                tone: DiffTone::Added,
                raw: new[j],
            });
            j += 1;
        }
    }
    extend(operations, &old[i..], DiffTone::Removed);
    extend(operations, &new[j..], DiffTone::Added);
    false
}

fn render_hunks(output: &mut Output, operations: &[Operation<'_>], context: usize) {
    let mut hunks = Vec::<(usize, usize)>::new();
    for (index, operation) in operations.iter().enumerate() {
        if operation.tone == DiffTone::Context {
            continue;
        }
        let start = index.saturating_sub(context);
        let end = index
            .saturating_add(context)
            .saturating_add(1)
            .min(operations.len());
        if let Some(last) = hunks.last_mut()
            && start <= last.1
        {
            last.1 = end;
        } else {
            hunks.push((start, end));
        }
    }
    let (mut cursor, mut old_seen, mut new_seen) = (0, 0, 0);
    for (start, end) in hunks {
        // No recontar prefijos por cada hunk: el recorrido total es lineal.
        for operation in &operations[cursor..start] {
            advance(operation.tone, &mut old_seen, &mut new_seen);
        }
        let old_count = operations[start..end]
            .iter()
            .filter(|operation| operation.tone != DiffTone::Added)
            .count();
        let new_count = operations[start..end]
            .iter()
            .filter(|operation| operation.tone != DiffTone::Removed)
            .count();
        let old_start = old_seen + usize::from(old_count != 0);
        let new_start = new_seen + usize::from(new_count != 0);
        output.push(
            format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@"),
            DiffTone::Hunk,
        );
        for operation in &operations[start..end] {
            let marker = match operation.tone {
                DiffTone::Context => ' ',
                DiffTone::Removed => '-',
                DiffTone::Added => '+',
                _ => unreachable!("las operaciones internas solo son contexto/alta/baja"),
            };
            output.push(format!("{marker}{}", body(operation.raw)), operation.tone);
            if !operation.raw.ends_with('\n') {
                output.push("\\ Sin salto de línea al final", DiffTone::Notice);
            }
            advance(operation.tone, &mut old_seen, &mut new_seen);
        }
        cursor = end;
    }
}

fn advance(tone: DiffTone, old: &mut usize, new: &mut usize) {
    *old += usize::from(tone != DiffTone::Added);
    *new += usize::from(tone != DiffTone::Removed);
}

fn body(raw: &str) -> String {
    let text = if let Some(text) = raw.strip_suffix("\r\n") {
        text
    } else {
        raw.strip_suffix('\n').unwrap_or(raw)
    };
    let mut visible = String::new();
    for character in text.chars() {
        if character.is_control() || character == '\u{feff}' {
            visible.extend(character.escape_default());
        } else {
            visible.push(character);
        }
    }
    visible
}

fn visible_escapes_needed(text: &str) -> bool {
    text.chars().any(|character| {
        character == '\u{feff}' || (character.is_control() && !matches!(character, '\r' | '\n'))
    }) || text
        .as_bytes()
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte == b'\r' && text.as_bytes().get(index + 1) != Some(&b'\n'))
}

fn endings(text: &str) -> String {
    if text.is_empty() {
        return "vacío".into();
    }
    let bytes = text.as_bytes();
    let mut crlf = false;
    let mut lf = false;
    let mut cr = false;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index != 0 && bytes[index - 1] == b'\r' {
                crlf = true;
            } else {
                lf = true;
            }
        } else if *byte == b'\r' && bytes.get(index + 1) != Some(&b'\n') {
            cr = true;
        }
    }
    let mut kinds = Vec::new();
    if crlf {
        kinds.push("CRLF");
    }
    if lf {
        kinds.push("LF");
    }
    if cr {
        kinds.push("CR aislado");
    }
    if kinds.is_empty() {
        kinds.push("ninguno");
    }
    format!(
        "{} · {}",
        kinds.join("/"),
        if text.ends_with('\n') {
            "con salto final"
        } else {
            "sin salto final"
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_tone(diff: &FormattedDiff, tone: DiffTone) -> Vec<&str> {
        diff.text
            .split('\n')
            .enumerate()
            .filter_map(|(index, line)| (diff.highlights[&index] == tone).then_some(line))
            .collect()
    }

    #[test]
    fn one_replacement_has_context_ranges_and_typed_markers() {
        let diff = format("a\nb\nc\nd\ne\n", "a\nb\nC\nd\ne\n", 1, 100);
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -2,3 +2,3 @@"]);
        assert_eq!(with_tone(&diff, DiffTone::Context), [" b", " d"]);
        assert_eq!(with_tone(&diff, DiffTone::Removed), ["-c"]);
        assert_eq!(with_tone(&diff, DiffTone::Added), ["+C"]);
        assert_eq!(diff.text.split('\n').count(), diff.highlights.len());
    }

    #[test]
    fn separate_changes_form_separate_hunks_without_copying_unchanged_middle() {
        let old = (0..20).map(|n| format!("line-{n}\n")).collect::<String>();
        let new = old
            .replace("line-2\n", "first\n")
            .replace("line-17\n", "last\n");
        let diff = format(&old, &new, 1, 1_000);
        assert_eq!(
            with_tone(&diff, DiffTone::Hunk),
            ["@@ -2,3 +2,3 @@", "@@ -17,3 +17,3 @@"]
        );
        assert!(!diff.text.contains("line-10"));
    }

    #[test]
    fn creation_and_deletion_use_zero_side_ranges() {
        let created = format("", "nuevo\n", 3, 0);
        assert_eq!(with_tone(&created, DiffTone::Hunk), ["@@ -0,0 +1,1 @@"]);
        assert_eq!(with_tone(&created, DiffTone::Added), ["+nuevo"]);
        let deleted = format("viejo\n", "", 3, 0);
        assert_eq!(with_tone(&deleted, DiffTone::Hunk), ["@@ -1,1 +0,0 @@"]);
        assert_eq!(with_tone(&deleted, DiffTone::Removed), ["-viejo"]);
    }

    #[test]
    fn zero_context_insertion_uses_preceding_line_position() {
        let diff = format("a\nc\n", "a\nb\nc\n", 0, 10);
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -1,0 +2,1 @@"]);
        assert_eq!(with_tone(&diff, DiffTone::Added), ["+b"]);
        assert!(with_tone(&diff, DiffTone::Context).is_empty());
    }

    #[test]
    fn zero_context_ranges_at_both_file_edges_count_only_source_lines() {
        for (before, after, expected) in [
            ("a\nb\n", "new\na\nb\n", "@@ -0,0 +1,1 @@"),
            ("a\nb\n", "a\nb\nnew\n", "@@ -2,0 +3,1 @@"),
            ("a\nb\n", "b\n", "@@ -1,1 +0,0 @@"),
            ("a\nb\n", "a\n", "@@ -2,1 +1,0 @@"),
        ] {
            let diff = format(before, after, 0, 100);
            assert_eq!(with_tone(&diff, DiffTone::Hunk), [expected]);
        }
    }

    #[test]
    fn final_newline_notice_does_not_increment_hunk_line_counts() {
        let diff = format("a", "a\n", 0, 4);
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -1,1 +1,1 @@"]);
        assert_eq!(
            with_tone(&diff, DiffTone::Notice)
                .iter()
                .filter(|line| line.starts_with("\\ Sin salto"))
                .count(),
            1
        );
    }

    #[test]
    fn repeated_lines_keep_minimal_deletion_and_correct_context() {
        let diff = format("x\nsame\nx\nsame\ny\n", "x\nsame\nsame\ny\n", 1, 100);
        assert_eq!(with_tone(&diff, DiffTone::Removed), ["-x"]);
        assert!(with_tone(&diff, DiffTone::Added).is_empty());
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -2,3 +2,2 @@"]);
    }

    #[test]
    fn terminators_are_compared_literally_and_missing_final_newline_is_visible() {
        let diff = format("uno\r\ndos\r\n", "uno\ndos", 0, 100);
        assert_eq!(with_tone(&diff, DiffTone::Removed), ["-uno", "-dos"]);
        assert_eq!(with_tone(&diff, DiffTone::Added), ["+uno", "+dos"]);
        assert!(diff.text.contains("Saltos antes: CRLF"));
        assert!(diff.text.contains("Saltos después: LF · sin salto final"));
        assert!(diff.text.contains("\\ Sin salto de línea al final"));
        assert!(!diff.text.contains('\r'));
    }

    #[test]
    fn identical_and_empty_inputs_never_invent_changed_lines() {
        for text in ["", "unchanged", "same\r\n", "\u{feff}igual\n"] {
            let diff = format(text, text, 3, 0);
            assert!(diff.text.contains("Sin cambios de contenido."));
            assert!(with_tone(&diff, DiffTone::Hunk).is_empty());
            assert!(with_tone(&diff, DiffTone::Added).is_empty());
            assert!(with_tone(&diff, DiffTone::Removed).is_empty());
        }
    }

    #[test]
    fn fallback_is_explicit_and_keeps_every_modified_source_line() {
        let before = "head\na\nshared\nb\ntail\n";
        let after = "head\nA\nshared\nB\ntail\n";
        let diff = format(before, after, 1, 1);
        assert!(diff.text.contains("Diff agrupado: límite de comparación"));
        assert_eq!(with_tone(&diff, DiffTone::Removed), ["-a", "-shared", "-b"]);
        assert_eq!(with_tone(&diff, DiffTone::Added), ["+A", "+shared", "+B"]);
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -1,5 +1,5 @@"]);
    }

    #[test]
    fn prefix_suffix_trimming_spends_budget_only_on_changed_middle() {
        let before = format!("{}old\n{}", "same\n".repeat(2_000), "tail\n".repeat(2_000));
        let after = before.replace("old\n", "new\n");
        let diff = format(&before, &after, 1, 4);
        assert!(!diff.text.contains("Diff agrupado"));
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -2000,3 +2000,3 @@"]);
        assert!(diff.text.len() < 500);
    }

    #[test]
    fn unicode_and_control_sequences_remain_data_without_any_raw_terminal_control() {
        let before = "\u{feff}á中🙂\t\u{1b}[31m\r";
        let after = "á中🙂\0\n";
        let diff = format(before, after, 3, 16);
        assert!(diff.text.contains("BOM UTF-8: antes sí; después no"));
        assert!(diff.text.contains("á中🙂"));
        assert!(diff.text.contains("\\t"));
        assert!(diff.text.contains("\\u{1b}[31m"));
        assert!(diff.text.contains("\\r"));
        assert!(!diff.text.chars().any(|c| c.is_control() && c != '\n'));
    }

    #[test]
    fn huge_context_saturates_and_retains_valid_ranges() {
        let diff = format("a\nb\nc\n", "a\nB\nc\n", usize::MAX, 4);
        assert_eq!(with_tone(&diff, DiffTone::Hunk), ["@@ -1,3 +1,3 @@"]);
    }

    #[test]
    fn bounded_middle_preserves_both_sources_for_all_short_repeated_sequences() {
        let mut sequences = vec![Vec::new()];
        for length in 1..=4 {
            for bits in 0..(1usize << length) {
                sequences.push(
                    (0..length)
                        .map(|bit| if bits & (1 << bit) == 0 { "a\n" } else { "b\n" })
                        .collect(),
                );
            }
        }
        for old in &sequences {
            for new in &sequences {
                for budget in [1, 25] {
                    let mut operations = Vec::new();
                    middle(old, new, budget, &mut operations);
                    let reconstructed_old: Vec<_> = operations
                        .iter()
                        .filter(|operation| operation.tone != DiffTone::Added)
                        .map(|operation| operation.raw)
                        .collect();
                    let reconstructed_new: Vec<_> = operations
                        .iter()
                        .filter(|operation| operation.tone != DiffTone::Removed)
                        .map(|operation| operation.raw)
                        .collect();
                    assert_eq!(&reconstructed_old, old);
                    assert_eq!(&reconstructed_new, new);
                }
            }
        }
    }
}
