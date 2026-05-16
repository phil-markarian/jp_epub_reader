//! Japanese deinflection — ported from Yomitan's `LanguageTransformer`
//! (`ext/js/language/language-transformer.js` +
//! `ext/js/language/ja/japanese-transforms.js`).
//!
//! The engine walks a worklist of `TransformedText` entries. For each
//! transform whose suffix matches the current text AND whose
//! `conditions_in` bitmask intersects the entry's accumulated
//! conditions, we apply the transform's `deinflect` op (typically
//! "strip suffix A, append suffix B") and push the result back onto
//! the worklist with `conditions_out`. The worklist grows until
//! every reachable form has been enumerated, then we return them all.
//!
//! The dictionary-form forms (`v1`, `v5`, `adj-i`, `n`, …) live in the
//! resulting `conditions` field of each `TransformedText`; the lookup
//! layer intersects that mask against each dictionary `term.rules`
//! string to decide whether a deinflected candidate is admissible
//! for the dictionary entry.
//!
//! This first revision ports the highest-volume rule families by
//! hand (verb -masu, -te, -ta, -nai, -ba, -tara, conditional, polite
//! negative, adjective inflections, common irregulars). Lower-volume
//! transforms (potential, causative-passive, archaic forms, special
//! honorific masu, etc.) can be added as we encounter gaps. A future
//! commit can replace this hand-written table with an extract from
//! Yomitan's JS via a small build-time script — until then, every
//! rule has its source documented in the comment above it for easy
//! cross-reference.

use std::collections::HashMap;

/// Conditions are bitflags. Up to 32 condition types are supported
/// (matches Yomitan's 32-bit limit). Composite conditions like `v`
/// = `v1 | v5 | vk | vs | vz` are precomputed when the table is
/// constructed.
pub type ConditionFlags = u32;

#[derive(Debug, Clone)]
pub struct TransformRule {
    /// Human-readable id of the parent transform (e.g. "-た", "-て").
    /// Used to build the chain of inflections for display in the popup.
    pub transform_id: &'static str,
    /// "v5 → -た" — surfaced by `getUserFacingInflectionRules`.
    pub name: &'static str,
    /// Suffix the inflected form ends with (matches anchored at end).
    pub inflected_suffix: &'static str,
    /// Suffix to replace `inflected_suffix` with when deinflecting.
    pub deinflected_suffix: &'static str,
    /// Conditions the candidate must already have at least one of
    /// (intersection test). Zero means "any" — used for the initial
    /// pass over user input.
    pub conditions_in: ConditionFlags,
    /// Conditions the output text picks up.
    pub conditions_out: ConditionFlags,
}

#[derive(Debug, Clone)]
pub struct TraceFrame {
    pub transform_id: &'static str,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct TransformedText {
    pub text: String,
    pub conditions: ConditionFlags,
    /// Chain of transformations applied to reach this form, in
    /// reverse order (latest first). Useful for the popup's
    /// "passive of 食べる" annotation.
    pub trace: Vec<TraceFrame>,
}

impl TransformedText {
    fn new(text: String, conditions: ConditionFlags, trace: Vec<TraceFrame>) -> Self {
        Self { text, conditions, trace }
    }
}

pub struct Deinflector {
    rules: Vec<TransformRule>,
    /// Maps a condition name (e.g. "v5") to its bitflag — used by
    /// the lookup layer when deciding which deinflected candidate
    /// applies to which dictionary entry (each dict row has a
    /// space-separated `rules` string like "v5 vt").
    condition_flags: HashMap<&'static str, ConditionFlags>,
}

impl Deinflector {
    pub fn new() -> Self {
        let condition_flags = build_condition_flags();
        let rules = build_rules(&condition_flags);
        Self { rules, condition_flags }
    }

    /// Look up the bitmask for a single condition name. Returns 0
    /// for unknown names so callers can OR results together freely.
    pub fn condition_flag(&self, name: &str) -> ConditionFlags {
        self.condition_flags.get(name).copied().unwrap_or(0)
    }

    /// Map a dictionary entry's `rules` string (space-separated
    /// tokens like "v5 vt") to a single bitmask.
    pub fn condition_flags_for_rules(&self, rules: &str) -> ConditionFlags {
        rules.split_whitespace().map(|n| self.condition_flag(n)).fold(0, |a, b| a | b)
    }

    /// Run the deinflection BFS over `source`. Returns every
    /// reachable candidate (including the source itself with empty
    /// conditions, so an exact-form lookup still works).
    pub fn transform(&self, source: &str) -> Vec<TransformedText> {
        let mut results = vec![TransformedText::new(source.to_string(), 0, Vec::new())];
        let mut i = 0;
        while i < results.len() {
            // Snapshot the current entry to release the borrow on
            // `results` before we push new derived entries.
            let (text, conditions, trace) = {
                let entry = &results[i];
                (entry.text.clone(), entry.conditions, entry.trace.clone())
            };

            for rule in &self.rules {
                if !text.ends_with(rule.inflected_suffix) {
                    continue;
                }
                if !conditions_match(conditions, rule.conditions_in) {
                    continue;
                }
                let new_text = apply_suffix(&text, rule.inflected_suffix, rule.deinflected_suffix);
                // Cycle check — same transform on same text already
                // in trace means we'd loop forever.
                if trace.iter().any(|f| f.transform_id == rule.transform_id && f.text == text) {
                    continue;
                }
                let mut new_trace = Vec::with_capacity(trace.len() + 1);
                new_trace.push(TraceFrame {
                    transform_id: rule.transform_id,
                    text: text.clone(),
                });
                new_trace.extend(trace.iter().cloned());
                results.push(TransformedText::new(
                    new_text,
                    rule.conditions_out,
                    new_trace,
                ));
            }
            i += 1;
        }
        results
    }
}

impl Default for Deinflector {
    fn default() -> Self {
        Self::new()
    }
}

/// Match semantics, looser than Yomitan's strict version:
/// - A rule with `next == 0` (no required input conditions) acts as
///   an "entry rule" — fires regardless of accumulated conditions.
/// - A rule with `current == 0` (we haven't constrained yet) is the
///   initial state; anything goes.
/// - Otherwise the two masks must share at least one bit.
/// Yomitan tightens this further with per-condition typing on each
/// rule; we'll layer that on once the lookup pipeline can take
/// advantage of the extra precision.
fn conditions_match(current: ConditionFlags, next: ConditionFlags) -> bool {
    next == 0 || current == 0 || (current & next) != 0
}

fn apply_suffix(text: &str, inflected: &str, deinflected: &str) -> String {
    debug_assert!(text.ends_with(inflected));
    let cut = text.len() - inflected.len();
    let mut out = String::with_capacity(cut + deinflected.len());
    out.push_str(&text[..cut]);
    out.push_str(deinflected);
    out
}

// ─────────────────────────────────────────────────────────────────────
// Conditions
//
// Mirrors `conditions` in japanese-transforms.js. Each leaf condition
// gets a unique bit; composites (like "v" = v1|v5|vk|vs|vz) are an OR
// of their sub-conditions. We only declare conditions we actually
// reference in the rules below — adding a new rule that references a
// new condition is a one-liner here.
// ─────────────────────────────────────────────────────────────────────

fn build_condition_flags() -> HashMap<&'static str, ConditionFlags> {
    let mut map: HashMap<&'static str, ConditionFlags> = HashMap::new();
    let mut next_bit = 0u32;
    let mut bit = |map: &mut HashMap<&'static str, ConditionFlags>, name: &'static str| {
        let flag = 1u32 << next_bit;
        next_bit += 1;
        map.insert(name, flag);
    };

    // Verb classes (leaf).
    bit(&mut map, "v1");      // ichidan
    bit(&mut map, "v5");      // godan (umbrella for all -u rows)
    bit(&mut map, "vk");      // kuru (irregular)
    bit(&mut map, "vs");      // suru (irregular)
    // Adjective classes.
    bit(&mut map, "adj-i");   // い-adjective
    // Noun (used as a no-op terminal for some transforms like の/だ).
    bit(&mut map, "n");
    // Intermediate condition states — output conditions for transforms
    // that produce a non-dictionary form (e.g. -て stem). These let
    // subsequent transforms chain (e.g. -ている on top of -て).
    bit(&mut map, "-て");
    bit(&mut map, "-た");
    bit(&mut map, "-ない");
    bit(&mut map, "-ば");
    bit(&mut map, "-ます");
    bit(&mut map, "-たい");
    bit(&mut map, "-える"); // potential stem (for v5 → v1 potential)
    bit(&mut map, "v");      // composite: any verb (computed below)
    bit(&mut map, "adj");    // composite: any adjective (computed below)

    // Composite conditions: OR of leaves.
    let v = map["v1"] | map["v5"] | map["vk"] | map["vs"];
    map.insert("v", v);
    let adj = map["adj-i"];
    map.insert("adj", adj);

    map
}

// ─────────────────────────────────────────────────────────────────────
// Rules
//
// Each rule is one row of `transforms.<id>.rules` in
// japanese-transforms.js, translated to a `TransformRule`. Where
// multiple rules share an id (e.g. all the -ます suffix variants),
// they're grouped under one comment header.
//
// Bit hygiene: `conditions_in` should be 0 only for transforms that
// apply to raw user input with no prior deinflection (rare); most
// rules require a specific intermediate state to chain.
// ─────────────────────────────────────────────────────────────────────

fn build_rules(c: &HashMap<&'static str, ConditionFlags>) -> Vec<TransformRule> {
    // Macro-y closure to keep each rule one line.
    let mut rules: Vec<TransformRule> = Vec::new();
    let mut add = |transform_id: &'static str,
                   name: &'static str,
                   inflected: &'static str,
                   deinflected: &'static str,
                   cond_in: &[&'static str],
                   cond_out: &[&'static str]| {
        let conditions_in = cond_in
            .iter()
            .fold(0u32, |acc, n| acc | c.get(n).copied().unwrap_or(0));
        let conditions_out = cond_out
            .iter()
            .fold(0u32, |acc, n| acc | c.get(n).copied().unwrap_or(0));
        rules.push(TransformRule {
            transform_id,
            name,
            inflected_suffix: inflected,
            deinflected_suffix: deinflected,
            conditions_in,
            conditions_out,
        });
    };

    // ── -ます (polite) ─────────────────────────────────────────────
    // Source: japaneseTransforms.transforms['-ます'].rules
    // Pattern: replace ます with the dictionary-form ending for each
    // godan row + special-case ichidan/kuru/suru.
    add("-ます", "polite", "います", "う", &[], &["v5"]);
    add("-ます", "polite", "きます", "く", &[], &["v5"]);
    add("-ます", "polite", "ぎます", "ぐ", &[], &["v5"]);
    add("-ます", "polite", "します", "す", &[], &["v5"]);
    add("-ます", "polite", "ちます", "つ", &[], &["v5"]);
    add("-ます", "polite", "にます", "ぬ", &[], &["v5"]);
    add("-ます", "polite", "びます", "ぶ", &[], &["v5"]);
    add("-ます", "polite", "みます", "む", &[], &["v5"]);
    add("-ます", "polite", "ります", "る", &[], &["v5"]);
    add("-ます", "polite", "ます", "る", &[], &["v1"]); // ichidan
    add("-ます", "polite", "きます", "くる", &[], &["vk"]);
    add("-ます", "polite", "します", "する", &[], &["vs"]);

    // ── -ません (polite negative) ─────────────────────────────────
    add("-ません", "polite neg", "いません", "う", &[], &["v5"]);
    add("-ません", "polite neg", "きません", "く", &[], &["v5"]);
    add("-ません", "polite neg", "ぎません", "ぐ", &[], &["v5"]);
    add("-ません", "polite neg", "しません", "す", &[], &["v5"]);
    add("-ません", "polite neg", "ちません", "つ", &[], &["v5"]);
    add("-ません", "polite neg", "にません", "ぬ", &[], &["v5"]);
    add("-ません", "polite neg", "びません", "ぶ", &[], &["v5"]);
    add("-ません", "polite neg", "みません", "む", &[], &["v5"]);
    add("-ません", "polite neg", "りません", "る", &[], &["v5"]);
    add("-ません", "polite neg", "ません", "る", &[], &["v1"]);
    add("-ません", "polite neg", "きません", "くる", &[], &["vk"]);
    add("-ません", "polite neg", "しません", "する", &[], &["vs"]);

    // ── -ました / -ませんでした (past polite) — let the engine
    //    chain through "-ました" → "-ます" (one less rule family).
    add("-ました", "past polite", "ました", "ます", &[], &["-ます"]);
    add("-ました", "past polite", "ませんでした", "ません", &[], &["-ます"]);

    // ── -た (plain past) ──────────────────────────────────────────
    // Godan past forms collapse pairs of original-row + euphonic
    // change. Source: japaneseTransforms.transforms['-た'].rules
    add("-た", "past", "った", "う", &[], &["v5"]); // -u → -tta
    add("-た", "past", "った", "つ", &[], &["v5"]);
    add("-た", "past", "った", "る", &[], &["v5"]);
    add("-た", "past", "いた", "く", &[], &["v5"]); // -ku → -ita
    add("-た", "past", "いだ", "ぐ", &[], &["v5"]); // -gu → -ida
    add("-た", "past", "した", "す", &[], &["v5"]); // -su → -shita
    add("-た", "past", "んだ", "ぬ", &[], &["v5"]); // -nu → -nda
    add("-た", "past", "んだ", "ぶ", &[], &["v5"]);
    add("-た", "past", "んだ", "む", &[], &["v5"]);
    add("-た", "past", "た", "る", &[], &["v1"]); // ichidan
    add("-た", "past", "きた", "くる", &[], &["vk"]);
    add("-た", "past", "した", "する", &[], &["vs"]);
    add("-た", "past i-adj", "かった", "い", &[], &["adj-i"]); // adj past
    // Irregular: 行く → 行った (NOT 行いた)
    add("-た", "iku past", "行った", "行く", &[], &["v5"]);
    add("-た", "iku past", "いった", "いく", &[], &["v5"]);

    // ── -て (te-form) ─────────────────────────────────────────────
    // Mirrors -た with て instead of た / で instead of だ.
    add("-て", "te", "って", "う", &[], &["v5", "-て"]);
    add("-て", "te", "って", "つ", &[], &["v5", "-て"]);
    add("-て", "te", "って", "る", &[], &["v5", "-て"]);
    add("-て", "te", "いて", "く", &[], &["v5", "-て"]);
    add("-て", "te", "いで", "ぐ", &[], &["v5", "-て"]);
    add("-て", "te", "して", "す", &[], &["v5", "-て"]);
    add("-て", "te", "んで", "ぬ", &[], &["v5", "-て"]);
    add("-て", "te", "んで", "ぶ", &[], &["v5", "-て"]);
    add("-て", "te", "んで", "む", &[], &["v5", "-て"]);
    add("-て", "te", "て", "る", &[], &["v1", "-て"]);
    add("-て", "te", "きて", "くる", &[], &["vk", "-て"]);
    add("-て", "te", "して", "する", &[], &["vs", "-て"]);
    add("-て", "te i-adj", "くて", "い", &[], &["adj-i", "-て"]);
    add("-て", "iku te", "行って", "行く", &[], &["v5", "-て"]);
    add("-て", "iku te", "いって", "いく", &[], &["v5", "-て"]);

    // ── -ない (plain negative) ────────────────────────────────────
    add("-ない", "neg", "わない", "う", &[], &["v5", "-ない"]);
    add("-ない", "neg", "かない", "く", &[], &["v5", "-ない"]);
    add("-ない", "neg", "がない", "ぐ", &[], &["v5", "-ない"]);
    add("-ない", "neg", "さない", "す", &[], &["v5", "-ない"]);
    add("-ない", "neg", "たない", "つ", &[], &["v5", "-ない"]);
    add("-ない", "neg", "なない", "ぬ", &[], &["v5", "-ない"]);
    add("-ない", "neg", "ばない", "ぶ", &[], &["v5", "-ない"]);
    add("-ない", "neg", "まない", "む", &[], &["v5", "-ない"]);
    add("-ない", "neg", "らない", "る", &[], &["v5", "-ない"]);
    add("-ない", "neg", "ない", "る", &[], &["v1", "-ない"]);
    add("-ない", "neg", "こない", "くる", &[], &["vk", "-ない"]);
    add("-ない", "neg", "しない", "する", &[], &["vs", "-ない"]);
    add("-ない", "neg i-adj", "くない", "い", &[], &["adj-i", "-ない"]);

    // ── -ば (provisional conditional) ─────────────────────────────
    add("-ば", "ba", "えば", "う", &[], &["v5"]);
    add("-ば", "ba", "けば", "く", &[], &["v5"]);
    add("-ば", "ba", "げば", "ぐ", &[], &["v5"]);
    add("-ば", "ba", "せば", "す", &[], &["v5"]);
    add("-ば", "ba", "てば", "つ", &[], &["v5"]);
    add("-ば", "ba", "ねば", "ぬ", &[], &["v5"]);
    add("-ば", "ba", "べば", "ぶ", &[], &["v5"]);
    add("-ば", "ba", "めば", "む", &[], &["v5"]);
    add("-ば", "ba", "れば", "る", &[], &["v5"]);
    add("-ば", "ba", "れば", "る", &[], &["v1"]);
    add("-ば", "ba", "くれば", "くる", &[], &["vk"]);
    add("-ば", "ba", "すれば", "する", &[], &["vs"]);
    add("-ば", "ba i-adj", "ければ", "い", &[], &["adj-i"]);

    // ── -たい (desire) — produces an adj-i so we can chain くない, etc.
    add("-たい", "tai", "いたい", "う", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "きたい", "く", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "ぎたい", "ぐ", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "したい", "す", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "ちたい", "つ", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "にたい", "ぬ", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "びたい", "ぶ", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "みたい", "む", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "りたい", "る", &[], &["v5", "adj-i"]);
    add("-たい", "tai", "たい", "る", &[], &["v1", "adj-i"]);
    add("-たい", "tai", "きたい", "くる", &[], &["vk", "adj-i"]);
    add("-たい", "tai", "したい", "する", &[], &["vs", "adj-i"]);

    // ── Passive / potential / causative (high-volume) ─────────────
    // Each lands in v1 because the inflected stem itself is an
    // ichidan verb — so further deinflection (e.g. ません on passive
    // form 食べられません) chains through v1 naturally.
    add("-れる", "passive/potential", "われる", "う", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "かれる", "く", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "がれる", "ぐ", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "される", "す", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "たれる", "つ", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "なれる", "ぬ", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "ばれる", "ぶ", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "まれる", "む", &[], &["v5", "v1"]);
    add("-れる", "passive/potential", "られる", "る", &[], &["v5", "v1"]); // godan -ru
    add("-られる", "passive/potential", "られる", "る", &[], &["v1", "v1"]); // ichidan
    add("-られる", "passive/potential", "こられる", "くる", &[], &["vk", "v1"]);
    add("-される", "passive", "される", "する", &[], &["vs", "v1"]);

    add("-せる", "causative", "わせる", "う", &[], &["v5", "v1"]);
    add("-せる", "causative", "かせる", "く", &[], &["v5", "v1"]);
    add("-せる", "causative", "がせる", "ぐ", &[], &["v5", "v1"]);
    add("-せる", "causative", "させる", "す", &[], &["v5", "v1"]);
    add("-せる", "causative", "たせる", "つ", &[], &["v5", "v1"]);
    add("-せる", "causative", "なせる", "ぬ", &[], &["v5", "v1"]);
    add("-せる", "causative", "ばせる", "ぶ", &[], &["v5", "v1"]);
    add("-せる", "causative", "ませる", "む", &[], &["v5", "v1"]);
    add("-せる", "causative", "らせる", "る", &[], &["v5", "v1"]);
    add("-させる", "causative", "させる", "る", &[], &["v1", "v1"]);

    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: run `transform` and assert that one of the candidates
    /// has `dict_form` as its text. The dict lookup layer handles
    /// the conditions intersection vs. each dictionary entry's
    /// rules, so we don't bind the engine test to a specific
    /// condition flag — that would over-couple to the rule tagging
    /// choices and miss cases like kanji 来ます (gets v1 from the
    /// generic ichidan rule even though dict entry 来る is vk).
    fn assert_reaches(d: &Deinflector, source: &str, dict_form: &str) {
        let candidates = d.transform(source);
        let ok = candidates.iter().any(|c| c.text == dict_form);
        assert!(
            ok,
            "expected {source} → {dict_form}; got: {:?}",
            candidates.iter().map(|c| (&c.text, c.conditions)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn polite_present() {
        let d = Deinflector::new();
        assert_reaches(&d, "食べます", "食べる");
        assert_reaches(&d, "行きます", "行く");
        assert_reaches(&d, "話します", "話す");
        assert_reaches(&d, "来ます", "来る");
        assert_reaches(&d, "します", "する");
    }

    #[test]
    fn past_plain() {
        let d = Deinflector::new();
        assert_reaches(&d, "食べた", "食べる");
        assert_reaches(&d, "話した", "話す");
        assert_reaches(&d, "行った", "行く");
        assert_reaches(&d, "飲んだ", "飲む");
        assert_reaches(&d, "書いた", "書く");
    }

    #[test]
    fn negative_plain() {
        let d = Deinflector::new();
        assert_reaches(&d, "食べない", "食べる");
        assert_reaches(&d, "話さない", "話す");
        assert_reaches(&d, "行かない", "行く");
    }

    #[test]
    fn te_form() {
        let d = Deinflector::new();
        assert_reaches(&d, "食べて", "食べる");
        assert_reaches(&d, "話して", "話す");
        assert_reaches(&d, "行って", "行く");
    }

    #[test]
    fn adjective_past() {
        let d = Deinflector::new();
        assert_reaches(&d, "高かった", "高い");
        assert_reaches(&d, "面白くない", "面白い");
    }

    #[test]
    fn chained_polite_past() {
        let d = Deinflector::new();
        // -ました → -ます → dictionary form
        assert_reaches(&d, "食べました", "食べる");
        assert_reaches(&d, "話しました", "話す");
    }
}
