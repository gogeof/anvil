//! Fuzzy matching algorithms for enhanced edit_file operations.
//!
//! Provides Levenshtein distance, Jaro-Winkler similarity, and other
//! fuzzy matching strategies to improve edit success rate.

use serde::{Deserialize, Serialize};
use std::cmp::{max, min};

/// Result of a fuzzy match operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyMatchResult {
    /// The matched text in the haystack.
    pub matched_text: String,
    /// Similarity score (0.0 to 1.0).
    pub similarity: f64,
    /// Start position of the match.
    pub start: usize,
    /// End position of the match.
    pub end: usize,
    /// Type of matching algorithm used.
    pub match_type: MatchType,
    /// Confidence level of the match.
    pub confidence: MatchConfidence,
    /// Description of differences from the needle.
    pub difference_description: Option<String>,
}

/// Type of fuzzy matching algorithm used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    /// Exact match.
    Exact,
    /// Match with whitespace normalization.
    WhitespaceNormalized,
    /// Match with case normalization.
    CaseNormalized,
    /// Match with both whitespace and case normalization.
    WhitespaceCaseNormalized,
    /// Match using Levenshtein distance.
    Levenshtein,
    /// Match using Jaro-Winkler similarity.
    JaroWinkler,
    /// Match using n-gram similarity.
    NGram,
    /// Match using longest common subsequence.
    LongestCommonSubsequence,
}

/// Confidence level of a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    /// Very high confidence (>= 95% similarity).
    VeryHigh,
    /// High confidence (>= 90% similarity).
    High,
    /// Medium confidence (>= 80% similarity).
    Medium,
    /// Low confidence (>= 70% similarity).
    Low,
    /// Very low confidence (< 70% similarity).
    VeryLow,
}

impl MatchConfidence {
    /// Determine confidence from similarity score.
    pub fn from_similarity(similarity: f64) -> Self {
        if similarity >= 0.95 {
            MatchConfidence::VeryHigh
        } else if similarity >= 0.90 {
            MatchConfidence::High
        } else if similarity >= 0.80 {
            MatchConfidence::Medium
        } else if similarity >= 0.70 {
            MatchConfidence::Low
        } else {
            MatchConfidence::VeryLow
        }
    }
}

/// Options for fuzzy matching.
#[derive(Debug, Clone)]
pub struct FuzzyMatchOptions {
    /// Minimum similarity threshold (0.0 to 1.0).
    pub min_similarity: f64,
    /// Maximum number of matches to return.
    pub max_matches: usize,
    /// Whether to normalize whitespace before matching.
    pub normalize_whitespace: bool,
    /// Whether to normalize case before matching.
    pub normalize_case: bool,
    /// Whether to use Levenshtein distance.
    pub use_levenshtein: bool,
    /// Whether to use Jaro-Winkler similarity.
    pub use_jaro_winkler: bool,
}

impl Default for FuzzyMatchOptions {
    fn default() -> Self {
        FuzzyMatchOptions {
            min_similarity: 0.7,
            max_matches: 5,
            normalize_whitespace: true,
            normalize_case: true,
            use_levenshtein: true,
            use_jaro_winkler: false,
        }
    }
}

/// Calculate Levenshtein distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();
    
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }
    
    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];
    
    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    
    for j in 0..=b_len {
        matrix[0][j] = j;
    }
    
    for (i, a_char) in a.chars().enumerate() {
        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            matrix[i + 1][j + 1] = min(
                min(
                    matrix[i][j + 1] + 1,     // deletion
                    matrix[i + 1][j] + 1,     // insertion
                ),
                matrix[i][j] + cost,          // substitution
            );
        }
    }
    
    matrix[a_len][b_len]
}

/// Calculate Levenshtein similarity (0.0 to 1.0).
pub fn levenshtein_similarity(a: &str, b: &str) -> f64 {
    let distance = levenshtein_distance(a, b);
    let max_len = max(a.chars().count(), b.chars().count());
    
    if max_len == 0 {
        return 1.0;
    }
    
    1.0 - (distance as f64 / max_len as f64)
}

/// Calculate Jaro similarity between two strings.
pub fn jaro_similarity(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    
    if a_len == 0 && b_len == 0 {
        return 1.0;
    }
    if a_len == 0 || b_len == 0 {
        return 0.0;
    }
    
    let match_distance = (max(a_len, b_len) / 2).saturating_sub(1);
    let mut a_matches = vec![false; a_len];
    let mut b_matches = vec![false; b_len];
    
    let mut matches = 0usize;
    let mut transpositions = 0usize;
    
    for i in 0..a_len {
        let start = max(0, i as isize - match_distance as isize) as usize;
        let end = min(i + match_distance + 1, b_len);
        
        for j in start..end {
            if b_matches[j] || a_chars[i] != b_chars[j] {
                continue;
            }
            a_matches[i] = true;
            b_matches[j] = true;
            matches += 1;
            break;
        }
    }
    
    if matches == 0 {
        return 0.0;
    }
    
    let mut k = 0;
    for i in 0..a_len {
        if !a_matches[i] {
            continue;
        }
        while !b_matches[k] {
            k += 1;
        }
        if a_chars[i] != b_chars[k] {
            transpositions += 1;
        }
        k += 1;
    }
    
    let matches = matches as f64;
    (matches / a_len as f64 + matches / b_len as f64 + (matches - transpositions as f64 / 2.0) / matches) / 3.0
}

/// Calculate Jaro-Winkler similarity between two strings.
pub fn jaro_winkler_similarity(a: &str, b: &str) -> f64 {
    let jaro = jaro_similarity(a, b);
    
    // Find common prefix length (max 4)
    let a_chars: Vec<char> = a.chars().take(4).collect();
    let b_chars: Vec<char> = b.chars().take(4).collect();
    
    let prefix_len = a_chars.iter().zip(b_chars.iter()).take_while(|(a, b)| a == b).count();
    
    // Jaro-Winkler adjustment
    let p = 0.1; // scaling factor
    jaro + prefix_len as f64 * p * (1.0 - jaro)
}

/// Normalize whitespace in a string.
pub fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Find all fuzzy matches of needle in haystack.
pub fn find_fuzzy_matches(haystack: &str, needle: &str, options: &FuzzyMatchOptions) -> Vec<FuzzyMatchResult> {
    let mut results = Vec::new();
    
    // Try exact match first
    if let Some(pos) = haystack.find(needle) {
        results.push(FuzzyMatchResult {
            matched_text: needle.to_string(),
            similarity: 1.0,
            start: pos,
            end: pos + needle.len(),
            match_type: MatchType::Exact,
            confidence: MatchConfidence::VeryHigh,
            difference_description: None,
        });
        return results;
    }
    
    // Try normalized matches
    let normalized_haystack = if options.normalize_whitespace {
        normalize_whitespace(haystack)
    } else {
        haystack.to_string()
    };
    
    let normalized_needle = if options.normalize_whitespace {
        normalize_whitespace(needle)
    } else {
        needle.to_string()
    };
    
    let case_normalized_haystack = normalized_haystack.to_lowercase();
    let case_normalized_needle = normalized_needle.to_lowercase();
    
    // Try case-normalized match
    if options.normalize_case {
        if let Some(pos) = case_normalized_haystack.find(&case_normalized_needle) {
            results.push(FuzzyMatchResult {
                matched_text: haystack[pos..pos + needle.len()].to_string(),
                similarity: 0.95,
                start: pos,
                end: pos + needle.len(),
                match_type: MatchType::CaseNormalized,
                confidence: MatchConfidence::VeryHigh,
                difference_description: Some("Case differs".to_string()),
            });
        }
    }
    
    // If no normalized matches found, try sliding window with Levenshtein
    if results.is_empty() && options.use_levenshtein {
        let needle_len = needle.chars().count();
        let haystack_chars: Vec<char> = haystack.chars().collect();
        
        // Sliding window approach
        let window_sizes = [
            needle_len,
            needle_len + 1,
            needle_len + 2,
            needle_len.saturating_sub(1),
            needle_len.saturating_sub(2),
        ];
        
        for window_size in window_sizes.iter().filter(|&&w| w > 0 && w <= haystack_chars.len()) {
            for i in 0..=(haystack_chars.len() - window_size) {
                let window: String = haystack_chars[i..i + window_size].iter().collect();
                let similarity = levenshtein_similarity(&window, needle);
                
                if similarity >= options.min_similarity {
                    let distance = levenshtein_distance(&window, needle);
                    results.push(FuzzyMatchResult {
                        matched_text: window.clone(),
                        similarity,
                        start: i,
                        end: i + window_size,
                        match_type: MatchType::Levenshtein,
                        confidence: MatchConfidence::from_similarity(similarity),
                        difference_description: Some(format!("Levenshtein distance: {}", distance)),
                    });
                }
            }
        }
    }
    
    // Sort by similarity (descending)
    results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
    
    // Limit results
    results.truncate(options.max_matches);
    
    results
}

/// Find the best fuzzy match of needle in haystack.
pub fn find_best_fuzzy_match(haystack: &str, needle: &str, options: &FuzzyMatchOptions) -> Option<FuzzyMatchResult> {
    let mut matches = find_fuzzy_matches(haystack, needle, options);
    matches.into_iter().next()
}

/// Describe the differences between two strings.
pub fn describe_differences(original: &str, modified: &str) -> String {
    let original_chars: Vec<char> = original.chars().collect();
    let modified_chars: Vec<char> = modified.chars().collect();
    
    let mut description = Vec::new();
    
    let len_diff = original_chars.len() as isize - modified_chars.len() as isize;
    if len_diff > 0 {
        description.push(format!("{} characters removed", len_diff));
    } else if len_diff < 0 {
        description.push(format!("{} characters added", -len_diff));
    }
    
    let distance = levenshtein_distance(original, modified);
    if distance > 0 {
        description.push(format!("edit distance: {}", distance));
    }
    
    if description.is_empty() {
        "identical".to_string()
    } else {
        description.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_distance_one_edit() {
        assert_eq!(levenshtein_distance("hello", "hallo"), 1);
        assert_eq!(levenshtein_distance("hello", "helo"), 1);
        assert_eq!(levenshtein_distance("hello", "helllo"), 1);
    }

    #[test]
    fn test_levenshtein_distance_multiple_edits() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn test_levenshtein_similarity() {
        assert!((levenshtein_similarity("hello", "hello") - 1.0).abs() < 0.001);
        assert!((levenshtein_similarity("hello", "hallo") - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_jaro_similarity_identical() {
        assert!((jaro_similarity("hello", "hello") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_jaro_winkler_similarity() {
        assert!(jaro_winkler_similarity("hello", "hello") >= jaro_similarity("hello", "hello"));
    }

    #[test]
    fn test_find_fuzzy_matches_exact() {
        let haystack = "Hello, world!";
        let needle = "world";
        let matches = find_fuzzy_matches(haystack, needle, &FuzzyMatchOptions::default());
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].match_type, MatchType::Exact);
    }

    #[test]
    fn test_find_fuzzy_matches_case_insensitive() {
        let haystack = "Hello, World!";
        let needle = "world";
        let options = FuzzyMatchOptions {
            normalize_case: true,
            ..Default::default()
        };
        let matches = find_fuzzy_matches(haystack, needle, &options);
        assert!(!matches.is_empty());
        assert!(matches[0].similarity >= 0.9);
    }

    #[test]
    fn test_find_fuzzy_matches_levenshtein() {
        let haystack = "Hello, wrld!";
        let needle = "world";
        let options = FuzzyMatchOptions {
            min_similarity: 0.7,
            use_levenshtein: true,
            ..Default::default()
        };
        let matches = find_fuzzy_matches(haystack, needle, &options);
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.match_type == MatchType::Levenshtein));
    }

    #[test]
    fn test_match_confidence_from_similarity() {
        assert_eq!(MatchConfidence::from_similarity(0.99), MatchConfidence::VeryHigh);
        assert_eq!(MatchConfidence::from_similarity(0.92), MatchConfidence::High);
        assert_eq!(MatchConfidence::from_similarity(0.85), MatchConfidence::Medium);
        assert_eq!(MatchConfidence::from_similarity(0.75), MatchConfidence::Low);
        assert_eq!(MatchConfidence::from_similarity(0.65), MatchConfidence::VeryLow);
    }

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_whitespace("hello   world"), "hello world");
        assert_eq!(normalize_whitespace("  hello  world  "), "hello world");
    }
}
