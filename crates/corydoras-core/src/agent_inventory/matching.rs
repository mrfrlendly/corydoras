use crate::inventory::InventoryDraft;

// Preserve nominal thread sizes and decimals. Split dimension separators and common
// unit suffixes so that M3×8 mm, M3 x 8mm, and M3x8 use the same size tokens.
pub(super) fn words(value: &str) -> Vec<String> {
    let lower = value.to_lowercase();
    let mut chars = lower.chars().peekable();
    let mut word = String::new();
    let mut result = Vec::new();
    let mut previous_digit = false;
    while let Some(c) = chars.next() {
        let after_digit = word.ends_with(|p: char| p.is_ascii_digit());
        let before_digit = chars.peek().is_some_and(|p| p.is_ascii_digit());
        let size_separator = c == 'x'
            && previous_digit
            && chars
                .clone()
                .find(|p| !p.is_whitespace())
                .is_some_and(|p| p.is_ascii_digit());
        if !c.is_whitespace() {
            previous_digit = c.is_ascii_digit();
        }
        if (c.is_alphanumeric() && !size_separator) || (c == '.' && after_digit && before_digit) {
            word.push(c);
        } else {
            finish_word(&mut result, &mut word);
        }
    }
    finish_word(&mut result, &mut word);
    result
}

fn finish_word(result: &mut Vec<String>, word: &mut String) {
    if word.is_empty() {
        return;
    }
    let numeric_end = word
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(word.len());
    if numeric_end > 0
        && matches!(
            &word[numeric_end..],
            "mm" | "cm" | "m" | "in" | "ft" | "g" | "kg" | "lb"
        )
    {
        result.push(word[..numeric_end].to_owned());
        result.push(word[numeric_end..].to_owned());
        word.clear();
    } else {
        result.push(std::mem::take(word));
    }
}

fn same_word(candidate: &str, term: &str) -> bool {
    candidate == term
        || (term.len() >= 3 && candidate.strip_suffix('s') == Some(term))
        || (candidate.len() >= 3 && term.strip_suffix('s') == Some(candidate))
}

// Imported notes can quote a catalogue URL or a parent kit with many different variants.
// Keep those lines for inspection, but exclude them from part and colour matching.
fn stock_notes(notes: &str) -> Vec<String> {
    notes
        .lines()
        .filter(|line| {
            let lower = line.trim_start().to_lowercase();
            !lower.starts_with("parent kit:")
                && !lower.starts_with("amazon (")
                && !lower.starts_with("catalogue availability:")
                && !lower.contains("://")
        })
        .flat_map(words)
        .collect()
}

pub(super) fn score(terms: &[String], draft: &InventoryDraft) -> Option<usize> {
    let fields = [
        (8, words(&draft.name)),
        (8, words(&draft.part_number)),
        (6, words(&draft.dimensions)),
        (6, words(&draft.colour)),
        (5, words(&draft.brand)),
        (4, words(&draft.category)),
        (1, stock_notes(&draft.notes)),
    ];
    let mut total = 0;
    for term in terms {
        let weight = fields
            .iter()
            .filter(|(_, field)| field.iter().any(|word| same_word(word, term)))
            .map(|(weight, _)| weight)
            .max()?;
        total += weight;
    }
    Some(total)
}
