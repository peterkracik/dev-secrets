//! A tiny fuzzy matcher used to filter lists, Telescope-style.
//!
//! [`score`] returns `None` when `query` is not a (case-insensitive)
//! subsequence of `text`, otherwise a score (higher is better) together with
//! the character indices in `text` that were matched, so the UI can highlight
//! them.

/// Score `text` against `query`. An empty query matches everything with a
/// neutral score and no highlights.
pub fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }

    let text_chars: Vec<char> = text.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();

    let mut matches = Vec::with_capacity(query_chars.len());
    let mut score = 0i32;
    let mut ti = 0usize;
    let mut last_match: Option<usize> = None;

    for &q in &query_chars {
        let ql = q.to_ascii_lowercase();
        let mut found = None;
        while ti < text_chars.len() {
            if text_chars[ti].to_ascii_lowercase() == ql {
                found = Some(ti);
                break;
            }
            ti += 1;
        }
        let pos = found?;

        // Bonus for matching at the very start.
        if pos == 0 {
            score += 15;
        }
        // Bonus for matching at a word boundary (after a separator).
        if pos > 0 {
            let prev = text_chars[pos - 1];
            if prev == '_' || prev == '-' || prev == '.' || prev == '/' || prev == ' ' {
                score += 10;
            }
        }
        // Bonus for consecutive matches; penalty for gaps.
        match last_match {
            Some(p) if p + 1 == pos => score += 10,
            Some(p) => score -= (pos - p - 1).min(10) as i32,
            None => score -= pos.min(10) as i32, // distance from start
        }

        matches.push(pos);
        last_match = Some(pos);
        ti = pos + 1;
    }

    // Shorter text that fully matches ranks slightly higher.
    score -= (text_chars.len() as i32) / 20;
    Some((score, matches))
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn empty_query_matches() {
        let (s, hl) = score("", "anything").unwrap();
        assert_eq!(s, 0);
        assert!(hl.is_empty());
    }

    #[test]
    fn non_subsequence_is_none() {
        assert!(score("xyz", "abc").is_none());
    }

    #[test]
    fn matches_subsequence_and_reports_indices() {
        let (_, hl) = score("db", "DATABASE_URL").unwrap();
        // 'd' at 0, 'b' somewhere after.
        assert_eq!(hl[0], 0);
        assert!(hl[1] > 0);
    }

    #[test]
    fn prefix_beats_scattered() {
        let prefix = score("api", "api_key").unwrap().0;
        let scattered = score("api", "a_p_i").unwrap().0;
        assert!(prefix > scattered);
    }
}
