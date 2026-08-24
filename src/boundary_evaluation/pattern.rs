pub fn is_valid_pattern(pattern: &str) -> bool {
    !pattern.is_empty()
        && pattern.split("::").all(|segment| {
            !segment.is_empty()
                && (segment == "*"
                    || segment == "**"
                    || (!segment.contains('*') && !segment.contains('?')))
        })
}

pub fn matches_pattern(pattern: &str, value: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split("::").collect();
    let value_segments: Vec<&str> = value.split("::").collect();
    match_segments(&pattern_segments, &value_segments)
}

fn match_segments(pattern: &[&str], value: &[&str]) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    if pattern[0] == "**" {
        (0..=value.len()).any(|index| match_segments(&pattern[1..], &value[index..]))
    } else if value.is_empty() {
        false
    } else if pattern[0] == "*" || pattern[0] == value[0] {
        match_segments(&pattern[1..], &value[1..])
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{is_valid_pattern, matches_pattern};

    #[test]
    fn should_accept_only_supported_pattern_segments() {
        // Arrange
        let accepted = ["crate", "crate::*", "crate::**::infra"];
        let rejected = ["", "crate::", "crate::app*", "crate::app?"];

        // Act
        let accepted_results = accepted.map(is_valid_pattern);
        let rejected_results = rejected.map(is_valid_pattern);

        // Assert
        assert!(accepted_results.into_iter().all(|result| result));
        assert!(rejected_results.into_iter().all(|result| !result));
    }

    #[test]
    fn should_match_single_and_multi_segment_wildcards() {
        // Arrange
        let cases = [
            ("crate::*::infra", "crate::app::infra", true),
            ("crate::**", "crate::app::infra", true),
            ("crate::**::infra", "crate::infra", true),
            ("crate::*", "crate::app::infra", false),
        ];

        // Act
        let results = cases.map(|(pattern, value, _)| matches_pattern(pattern, value));

        // Assert
        for ((_, _, expected), actual) in cases.into_iter().zip(results) {
            assert_eq!(actual, expected);
        }
    }
}
