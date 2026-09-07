//! Reconciling prose against the ledger (§6.2).
//!
//! The runtime can guarantee its own output. It cannot guarantee what an agent writes
//! afterwards — a transposed digit, a wrong unit, a total the model helpfully recomputed, a
//! figure carried over from an earlier turn. All the contract work sits upstream of where
//! fabrication actually happens, and this module is what closes it.
//!
//! **No model is involved.** Every numeral in the text is extracted and checked against the
//! numbers the ledger recorded. It is string and number handling, nothing more.
//!
//! Order matters: a numeral is matched against the ledger *first*, and only if that fails is
//! it tested against the ignore rules. That way an over-broad ignore rule can never suppress
//! a figure that genuinely came from a node — it can only soften a miss.

use serde_json::Value as Json;
use std::collections::BTreeMap;

/// A number as it appears in prose, with everything needed to compare it fairly.
#[derive(Debug, Clone, PartialEq)]
pub struct Numeral {
    /// Exactly as written, including separators and sign.
    pub raw: String,
    pub value: f64,
    /// Digits written after the decimal point. `512` has 0; `512.4` has 1.
    pub precision: usize,
    pub percent: bool,
    /// A currency symbol before, or a word or `%` immediately after. `40 kg` has one; a bare
    /// `40` does not.
    pub has_unit: bool,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug)]
pub struct Unmatched {
    pub numeral: Numeral,
    pub context: String,
}

/// A numeral that the ledger accounts for, and every recorded value that accounts for it.
///
/// The plural is the point. Attestation asks *does any recorded number round to this?*, so a
/// figure matched by several unrelated values is matched just as firmly as one matched by
/// exactly the value it is about — and the report said "matched" for both. That is the hole
/// the mutation sweep found: it catches 23 of 24 corruptions of a real answer, and the
/// survivor is a wrong value that happens to collide with an unrelated figure elsewhere in
/// the ledger. It degrades with ledger size exactly as you would expect: in a two-call ledger
/// 19% of the integers 1 to 99 are already present, in a day's 96%.
///
/// Naming the paths does not close that hole. It makes it visible, which is the difference
/// between a check that can be audited and one that can only be trusted: `512 AR` accounted
/// for by `result.attack_rating` reads differently from the same numeral accounted for by
/// `result.rows[7].weight`, and only one of those is worth acting on.
#[derive(Debug)]
pub struct Accounted {
    pub numeral: Numeral,
    /// Dotted ledger paths, prefixed by the entry they came from. Sorted, and capped — a long
    /// ledger can account for a small integer many times over and the list stops being
    /// evidence once it is a page long.
    pub paths: Vec<String>,
    /// How many paths there were before the cap.
    pub accounted_by: usize,
}

/// The most paths listed for one numeral. Past this the list has stopped being evidence and
/// started being a census; `accounted_by` still carries the real number.
const MAX_PATHS: usize = 8;

#[derive(Debug)]
pub struct Report {
    pub checked: usize,
    pub matched: usize,
    pub ignored: usize,
    pub unmatched: Vec<Unmatched>,
    /// Every matched numeral and what accounted for it, in the order they appear in the text.
    pub accounted: Vec<Accounted>,
}

impl Report {
    /// Numerals that more than one recorded value could account for.
    ///
    /// A count of how loose this particular attestation was. Zero means every figure in the
    /// prose traces to exactly one thing the ledger recorded, which is the strong reading of
    /// "attested"; a high number against a large ledger means the check passed for reasons
    /// that may have nothing to do with the answer.
    pub fn ambiguous(&self) -> usize {
        self.accounted.iter().filter(|a| a.accounted_by > 1).count()
    }
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.unmatched.is_empty()
    }
}

// ------------------------------------------------------------------- extraction

/// Short words that follow a number without being a unit for it. Without these, `3 of them`
/// looks like three "of", and the ordinal rule never fires.
const NOT_UNITS: &[&str] = &[
    "of", "in", "the", "a", "an", "and", "or", "to", "for", "at", "on", "by", "is", "are", "was",
    "it", "its", "no", "not", "if", "as", "so", "but", "out", "up", "off", "who",
];

/// Whether the text right after a number reads as that number's unit.
///
/// This is a heuristic, and deliberately a conservative one. It only decides which
/// *unmatched* numerals get excused by the ignore rules — a numeral that the ledger accounts
/// for is matched before any of this is consulted. So a missed unit costs a little detection
/// in a narrow band; a spurious one would suppress the ignore rules on ordinary prose, which
/// is far worse.
///
/// A unit is either glued on (`40kg`, `25%`) or a short word after a single space (`40 kg`,
/// `512 AR`). A longer word is prose, not a unit.
fn has_unit_suffix(tail: &str) -> bool {
    const MAX_UNIT_LEN: usize = 4;

    if tail.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return true;
    }
    let Some(rest) = tail.strip_prefix(' ') else {
        return false;
    };
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();

    !word.is_empty()
        && word.len() <= MAX_UNIT_LEN
        && !NOT_UNITS.contains(&word.to_ascii_lowercase().as_str())
}

fn is_sign_boundary(byte: u8) -> bool {
    // A `-` counts as a sign only when nothing word-like precedes it, so `T-1001` yields
    // 1001 rather than -1001, while `remaining -250` yields -250.
    !byte.is_ascii_alphanumeric() && byte != b'.' && byte != b'-'
}

/// Pull every number out of a block of text.
pub fn extract(text: &str) -> Vec<Numeral> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }

        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }

        // Thousands separators, but only in the shape a separator actually takes: a comma
        // followed by exactly three digits. `1,234` is one number; `1, 234` is two.
        while i + 3 < bytes.len()
            && bytes[i] == b','
            && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit)
            && !bytes.get(i + 4).is_some_and(u8::is_ascii_digit)
        {
            i += 4;
        }

        let mut precision = 0;
        if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
            i += 1;
            let fraction_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            precision = i - fraction_start;
        }

        // Reclaim a leading sign if one is really acting as a sign.
        let mut start = digits_start;
        if digits_start > 0 && matches!(bytes[digits_start - 1], b'-' | b'+') {
            let before_sign = digits_start
                .checked_sub(2)
                .map(|j| bytes[j])
                .unwrap_or(b' ');
            if is_sign_boundary(before_sign) {
                start = digits_start - 1;
            }
        }

        let raw = &text[start..i];
        let Ok(value) = raw.replace(',', "").parse::<f64>() else {
            continue;
        };

        // The character before the numeral, not the byte: `£` and `€` are multi-byte, so
        // slicing one byte back both failed to match them and panicked on any multi-byte
        // character that happened to sit there (`±5`, `—3`).
        let prefix_unit = text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| matches!(c, '$' | '£' | '€' | '¥'));

        let tail = &text[i..];
        let percent = tail.trim_start_matches(' ').starts_with('%');
        let suffix_unit = percent || has_unit_suffix(tail);

        let (line, column) = line_and_column(text, start);
        out.push(Numeral {
            raw: raw.to_string(),
            value,
            precision,
            percent,
            has_unit: prefix_unit || suffix_unit,
            offset: start,
            line,
            column,
        });
    }
    out
}

fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rfind('\n').map_or(offset, |nl| offset - nl - 1) + 1;
    (line, column)
}

fn context(text: &str, numeral: &Numeral) -> String {
    const WINDOW: usize = 36;
    let start = text[..numeral.offset]
        .char_indices()
        .rev()
        .take(WINDOW)
        .last()
        .map_or(0, |(i, _)| i);
    let after = numeral.offset + numeral.raw.len();
    let end = text[after..]
        .char_indices()
        .take(WINDOW)
        .last()
        .map_or(after, |(i, c)| after + i + c.len_utf8());

    let snippet = text[start..end].replace('\n', " ");
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        snippet.trim(),
        if end < text.len() { "…" } else { "" }
    )
}

// --------------------------------------------------------------------- matching

/// Round a ledger value to the precision the prose was written at, per §6.2: `512` matches a
/// recorded `512.4` because the writer rounded, not because they made it up.
fn rounds_to(scalar: f64, value: f64, precision: usize) -> bool {
    let factor = 10f64.powi(precision as i32);
    let rounded = (scalar * factor).round() / factor;
    (rounded - value).abs() <= 1e-9 * value.abs().max(1.0)
}

/// Every recorded path whose value could account for this numeral.
///
/// Empty means unmatched. More than one means the match is ambiguous, which is worth knowing
/// and was previously indistinguishable from a single exact hit.
fn matching_paths(numeral: &Numeral, scalars: &BTreeMap<String, f64>) -> Vec<String> {
    // A percent in prose can stand for either form of the recorded figure: `12.3%` is a fair
    // rendering of both 12.3 and 0.123. The divided form is checked at two more decimal
    // places, since that is the precision dividing by a hundred implies.
    let mut candidates = vec![(numeral.value, numeral.precision)];
    if numeral.percent {
        candidates.push((numeral.value / 100.0, numeral.precision + 2));
    }

    scalars
        .iter()
        .filter(|(_, scalar)| {
            candidates
                .iter()
                .any(|(value, precision)| rounds_to(**scalar, *value, *precision))
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// Reasons a numeral need not appear in the ledger. Checked only after matching has failed,
/// so these can never hide a figure that did come from a node.
fn ignorable(numeral: &Numeral, question_values: &[f64]) -> bool {
    // Quoting the user's own question back at them is not fabrication.
    if question_values
        .iter()
        .any(|q| (q - numeral.value).abs() < f64::EPSILON)
    {
        return true;
    }
    if numeral.has_unit {
        return false;
    }
    // A bare four-digit year.
    if numeral.precision == 0 && (1000.0..=2999.0).contains(&numeral.value) {
        return true;
    }
    // Small bare integers: ordinals, list counts, "one or two things".
    if numeral.precision == 0 && (0.0..=10.0).contains(&numeral.value) {
        return true;
    }
    false
}

/// Check every numeral in `text` against the ledger's recorded values.
///
/// Takes the whole map rather than its values, so a match can say *what* accounted for it.
pub fn attest(text: &str, scalars: &BTreeMap<String, f64>, question: Option<&str>) -> Report {
    let question_values: Vec<f64> = question
        .map(|q| extract(q).iter().map(|n| n.value).collect())
        .unwrap_or_default();

    let mut report = Report {
        checked: 0,
        matched: 0,
        ignored: 0,
        unmatched: Vec::new(),
        accounted: Vec::new(),
    };

    for numeral in extract(text) {
        report.checked += 1;
        let mut paths = matching_paths(&numeral, scalars);
        if !paths.is_empty() {
            report.matched += 1;
            let accounted_by = paths.len();
            paths.truncate(MAX_PATHS);
            report.accounted.push(Accounted {
                numeral,
                paths,
                accounted_by,
            });
        } else if ignorable(&numeral, &question_values) {
            report.ignored += 1;
        } else {
            let context = context(text, &numeral);
            report.unmatched.push(Unmatched { numeral, context });
        }
    }
    report
}

/// Every scalar the ledger recorded, ready to check against.
///
/// By default only `result.*` values count. Inputs are recorded in the ledger and can be
/// admitted with `include_inputs`, but they are not provenanced the way a result is: an agent
/// chose them, so treating them as verified would launder a fabricated argument into an
/// attested one.
pub fn ledger_scalars(entries: &[Json], include_inputs: bool) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();

    for (i, entry) in entries.iter().enumerate() {
        if let Some(scalars) = entry.get("scalars").and_then(Json::as_object) {
            for (key, value) in scalars {
                if let Some(number) = value.as_f64() {
                    out.insert(format!("{i}:{key}"), number);
                }
            }
        }
        if include_inputs {
            if let Some(input) = entry.get("input") {
                for (key, value) in crate::ledger::scalars(input, "input") {
                    if let Some(number) = value.as_f64() {
                        out.insert(format!("{i}:{key}"), number);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(text: &str) -> Vec<f64> {
        extract(text).iter().map(|n| n.value).collect()
    }

    /// A ledger of the given values, under throwaway paths. These tests are about matching
    /// and rounding, not about provenance, so the paths only have to be distinct.
    fn ledger(scalars: &[f64]) -> BTreeMap<String, f64> {
        scalars
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("0:result.f{i}"), *v))
            .collect()
    }

    // ----------------------------------------------------------------- extraction

    /// Found by the acceptance run (§10 step 6): a model wrote "±5" and extraction panicked
    /// slicing one byte back from the digit, which landed inside the `±`. The same line could
    /// never have matched `£`, `€` or `¥` either, since none of them is one byte.
    #[test]
    fn a_multi_byte_character_before_a_numeral_is_handled() {
        for text in ["AR figures are ±5", "roughly —3 points", "→7 of them", "±5"] {
            let _ = extract(text);
        }
        assert_eq!(values("AR figures are from memory, ±5"), vec![5.0]);

        // A currency prefix marks the number as a measured quantity, which is what keeps the
        // small-integer ignore rule from excusing it.
        for (text, symbol) in [("$5", "$"), ("£5", "£"), ("€5", "€"), ("¥5", "¥")] {
            let found = extract(text);
            assert_eq!(found.len(), 1, "{text}");
            assert!(found[0].has_unit, "{symbol} should mark {text} as measured");
        }
        assert!(!extract("x5").first().unwrap().has_unit);
    }

    #[test]
    fn finds_plain_numbers() {
        assert_eq!(
            values("the count is 3 and the length is 10"),
            vec![3.0, 10.0]
        );
    }

    #[test]
    fn reads_thousands_separators_but_not_list_commas() {
        assert_eq!(values("1,234.5 spent"), vec![1234.5]);
        // A comma with a space after it is punctuation, not a separator.
        assert_eq!(values("items 1, 234 and 5"), vec![1.0, 234.0, 5.0]);
        // Four digits after a comma is not a thousands group.
        assert_eq!(values("1,2345"), vec![1.0, 2345.0]);
    }

    #[test]
    fn treats_a_leading_dash_as_a_sign_only_at_a_word_boundary() {
        assert_eq!(values("remaining -250 minutes"), vec![-250.0]);
        // An identifier must not become a negative number.
        assert_eq!(values("ticket T-1001 is open"), vec![1001.0]);
    }

    #[test]
    fn records_precision_as_written() {
        let found = extract("190.9 and 512 and 0.123");
        assert_eq!(found[0].precision, 1);
        assert_eq!(found[1].precision, 0);
        assert_eq!(found[2].precision, 3);
    }

    #[test]
    fn spots_units_before_and_after() {
        assert!(
            extract("$2000 monthly").first().unwrap().has_unit,
            "currency prefix"
        );
        assert!(extract("40 kg").first().unwrap().has_unit, "spaced unit");
        assert!(extract("512 AR").first().unwrap().has_unit, "spaced unit");
        assert!(extract("40kg").first().unwrap().has_unit, "glued unit");
        assert!(extract("25% credit").first().unwrap().percent);
    }

    /// Ordinary prose after a number is not a unit. If it were, every number in a sentence
    /// would look measured and the ignore rules would never fire.
    #[test]
    fn following_prose_is_not_mistaken_for_a_unit() {
        assert!(
            !extract("in 2026 there were three")
                .first()
                .unwrap()
                .has_unit
        );
        assert!(!extract("3 of them").first().unwrap().has_unit);
        assert!(
            !extract("7 items").first().unwrap().has_unit,
            "'items' is too long to be a unit"
        );
        assert!(!extract("there were 7").first().unwrap().has_unit);
    }

    // ------------------------------------------------------------------- matching

    #[test]
    fn an_exact_figure_matches() {
        assert!(attest("the credit is 500", &ledger(&[500.0]), None).is_clean());
    }

    #[test]
    fn a_rounded_figure_matches_the_recorded_one() {
        // §6.2: 512 matches a recorded 512.4.
        assert!(attest("about 512 AR", &ledger(&[512.4]), None).is_clean());
        assert!(attest("190.9 exactly", &ledger(&[190.9]), None).is_clean());
    }

    #[test]
    fn a_wrong_digit_does_not_match() {
        let report = attest("an attack rating of 512.9", &ledger(&[512.4]), None);
        assert_eq!(report.unmatched.len(), 1);
        assert_eq!(report.unmatched[0].numeral.raw, "512.9");
    }

    #[test]
    fn a_transposed_digit_is_caught() {
        let report = attest("the credit is $509", &ledger(&[500.0]), None);
        assert_eq!(report.unmatched.len(), 1, "509 is not 500");
    }

    #[test]
    fn percent_matches_both_forms() {
        assert!(attest("a 12.3% credit", &ledger(&[12.3]), None).is_clean());
        assert!(attest("a 12.3% credit", &ledger(&[0.123]), None).is_clean());
    }

    #[test]
    fn thousands_separators_match_the_bare_figure() {
        assert!(attest("1,234.5 total", &ledger(&[1234.5]), None).is_clean());
    }

    // -------------------------------------------------------------- ignore rules

    #[test]
    fn numbers_from_the_question_are_not_fabrication() {
        let report = attest(
            "at soul level 120 you get 42",
            &ledger(&[42.0]),
            Some("build at SL120?"),
        );
        assert!(report.is_clean(), "{:?}", report.unmatched);
        assert_eq!(report.ignored, 1);
    }

    #[test]
    fn bare_years_and_small_integers_are_ignored() {
        let report = attest("in 2026 there were 3 of them", &ledger(&[]), None);
        assert!(report.is_clean(), "{:?}", report.unmatched);
        assert_eq!(report.ignored, 2);
    }

    /// The ignore rules must not swallow a figure that carries a unit — `$2000` is a claim
    /// about money, not a year.
    #[test]
    fn a_year_shaped_number_with_a_unit_is_still_checked() {
        let report = attest("a $2000 monthly fee", &ledger(&[]), None);
        assert_eq!(
            report.unmatched.len(),
            1,
            "a currency figure is never a year"
        );
    }

    /// Matching runs before the ignore rules, so a real figure is never merely "ignored".
    #[test]
    fn matching_takes_precedence_over_ignoring() {
        let report = attest("strength 10", &ledger(&[10.0]), None);
        assert_eq!(report.matched, 1);
        assert_eq!(report.ignored, 0);
    }

    #[test]
    fn positions_are_reported() {
        let report = attest("line one\nthe value is 999 here", &ledger(&[1.0]), None);
        let found = &report.unmatched[0];
        assert_eq!(found.numeral.line, 2);
        assert_eq!(found.numeral.column, 14);
        assert!(found.context.contains("999"));
    }
}
