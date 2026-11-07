//! Recurring requests: grouping request fingerprints and judging which groups
//! are worth reporting.
//!
//! Pure. Where the fingerprints come from and how similar texts are found
//! quickly is the store's business; this module only decides which texts are
//! the same request and when a group has recurred enough.
//!
//! Grouping is two steps. Fingerprints with an identical `normalized` text form
//! one **seed**. Two seeds join when the Jaccard similarity of their token sets
//! is at least [`MIN_SIMILARITY`], with single linkage: a chain of similar
//! seeds is one group even when its ends are not similar to each other.

use std::collections::{BTreeMap, BTreeSet};

/// How many days back a fingerprint counts. The same 90 days the memory sweep
/// keeps fingerprints for.
pub const WINDOW_DAYS: u64 = 90;
/// The smallest Jaccard similarity of two token sets for their seeds to join.
pub const MIN_SIMILARITY: f64 = 0.6;
/// Distinct sessions a group needs to be a candidate. Repeats inside one
/// session count once.
pub const MIN_SESSIONS: usize = 3;
/// Distinct days those sessions must span.
pub const MIN_DAYS: usize = 2;
/// The most sample texts a candidate carries.
pub const MAX_SAMPLES: usize = 3;

/// The placeholders [`crate::domain::fingerprint::normalize`] writes. Each is
/// one token, so `<path>` is not confused with the word `path`.
const PLACEHOLDERS: [&str; 4] = ["<path>", "<id>", "<n>", "<token>"];

/// One stored request fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub id: i64,
    pub observation_id: i64,
    /// Empty when the prompt was typed outside an `ivar` session.
    pub session_id: String,
    pub feature: String,
    /// The UTC date of the prompt, `YYYY-MM-DD`.
    pub day: String,
    pub normalized: String,
}

impl Fingerprint {
    /// Orders fingerprints by how recent they are: the day, then the id, which
    /// grows with every insert.
    #[must_use]
    pub fn recency(&self) -> (&str, i64) {
        (self.day.as_str(), self.id)
    }
}

/// The tokens of a normalized text: runs of letters and digits, and the
/// placeholders that normalization wrote.
#[must_use]
pub fn tokens(normalized: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut word = String::new();
    let mut rest = normalized;
    while let Some(c) = rest.chars().next() {
        let placeholder = PLACEHOLDERS
            .iter()
            .find_map(|p| rest.strip_prefix(p).map(|after| (*p, after)));
        if let Some((placeholder, after)) = placeholder {
            flush(&mut word, &mut found);
            found.insert(placeholder.to_owned());
            rest = after;
            continue;
        }
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut found);
        }
        rest = rest.strip_prefix(c).unwrap_or_default();
    }
    flush(&mut word, &mut found);
    found
}

fn flush(word: &mut String, into: &mut BTreeSet<String>) {
    if !word.is_empty() {
        into.insert(std::mem::take(word));
    }
}

/// `|a ∩ b| / |a ∪ b|`; zero when both sets are empty.
#[must_use]
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        a.intersection(b).count() as f64 / union as f64
    }
}

/// Fingerprints that are the same recurring request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Most recent first.
    members: Vec<Fingerprint>,
}

impl Group {
    /// A group of `members`; the order they are given in does not matter.
    #[must_use]
    pub fn new(mut members: Vec<Fingerprint>) -> Self {
        members.sort_by(|a, b| b.recency().cmp(&a.recency()));
        Self { members }
    }

    /// Every fingerprint of the group, most recent first.
    #[must_use]
    pub fn members(&self) -> &[Fingerprint] {
        &self.members
    }

    /// The most recent fingerprint.
    #[must_use]
    pub fn latest(&self) -> Option<&Fingerprint> {
        self.members.first()
    }

    /// The prompts that were typed in an `ivar` session. A prompt outside one
    /// cannot be told apart from another, so it is no evidence of a session.
    fn in_sessions(&self) -> impl Iterator<Item = &Fingerprint> {
        self.members.iter().filter(|f| !f.session_id.is_empty())
    }

    /// The distinct sessions, sorted.
    #[must_use]
    pub fn sessions(&self) -> BTreeSet<&str> {
        self.in_sessions().map(|f| f.session_id.as_str()).collect()
    }

    /// The distinct days those sessions' prompts were typed on, sorted.
    #[must_use]
    pub fn days(&self) -> BTreeSet<&str> {
        self.in_sessions().map(|f| f.day.as_str()).collect()
    }

    /// The distinct non-empty features, sorted.
    #[must_use]
    pub fn features(&self) -> BTreeSet<&str> {
        self.members
            .iter()
            .map(|f| f.feature.as_str())
            .filter(|feature| !feature.is_empty())
            .collect()
    }

    /// Every distinct normalized text of the group, sorted.
    #[must_use]
    pub fn texts(&self) -> BTreeSet<&str> {
        self.members.iter().map(|f| f.normalized.as_str()).collect()
    }

    /// At most `max` distinct normalized texts, most recent first.
    #[must_use]
    pub fn samples(&self, max: usize) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.members
            .iter()
            .map(|f| f.normalized.as_str())
            .filter(|text| seen.insert(*text))
            .take(max)
            .collect()
    }

    /// Whether the group recurred: [`MIN_SESSIONS`] distinct sessions on at
    /// least [`MIN_DAYS`] distinct days.
    #[must_use]
    pub fn is_candidate(&self) -> bool {
        self.sessions().len() >= MIN_SESSIONS && self.days().len() >= MIN_DAYS
    }
}

/// Groups `fingerprints`. A fingerprint whose text normalized to nothing says
/// nothing about what was asked and is left out.
///
/// `similar` lists the normalized texts that may be similar to a seed whose
/// token set it is given, for instance every stored text that shares a token.
/// It may return too many (each answer is checked against [`MIN_SIMILARITY`]
/// here) but a text it misses can never join the seed. Texts it returns that
/// no fingerprint holds are ignored.
///
/// # Errors
///
/// The first error `similar` returns.
pub fn group<E>(
    fingerprints: Vec<Fingerprint>,
    mut similar: impl FnMut(&BTreeSet<String>) -> Result<Vec<String>, E>,
) -> Result<Vec<Group>, E> {
    let mut seeds: BTreeMap<String, Vec<Fingerprint>> = BTreeMap::new();
    for fingerprint in fingerprints {
        if !fingerprint.normalized.is_empty() {
            seeds
                .entry(fingerprint.normalized.clone())
                .or_default()
                .push(fingerprint);
        }
    }
    let texts: Vec<&String> = seeds.keys().collect();
    let position: BTreeMap<&str, usize> = texts
        .iter()
        .enumerate()
        .map(|(at, text)| (text.as_str(), at))
        .collect();
    let token_sets: Vec<BTreeSet<String>> = texts.iter().map(|text| tokens(text)).collect();

    let mut union_find = UnionFind::new(texts.len());
    for (at, set) in token_sets.iter().enumerate() {
        for text in similar(set)? {
            // Similarity is symmetric, so each pair is judged once.
            let Some(&other) = position.get(text.as_str()).filter(|&&other| other > at) else {
                continue;
            };
            let linked = token_sets
                .get(other)
                .is_some_and(|theirs| jaccard(set, theirs) >= MIN_SIMILARITY);
            if linked {
                union_find.union(at, other);
            }
        }
    }

    let mut by_root: BTreeMap<usize, Vec<Fingerprint>> = BTreeMap::new();
    for (at, (_, members)) in seeds.into_iter().enumerate() {
        by_root
            .entry(union_find.find(at))
            .or_default()
            .extend(members);
    }
    Ok(by_root.into_values().map(Group::new).collect())
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, mut at: usize) -> usize {
        while let Some(&up) = self.parent.get(at) {
            if up == at {
                break;
            }
            // Path halving.
            let grand = self.parent.get(up).copied().unwrap_or(up);
            if let Some(slot) = self.parent.get_mut(at) {
                *slot = grand;
            }
            at = grand;
        }
        at
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b
            && let Some(slot) = self.parent.get_mut(a.max(b))
        {
            *slot = a.min(b);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/repetition.rs"]
mod tests;
