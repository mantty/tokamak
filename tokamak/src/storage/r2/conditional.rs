//! Conditions on an object's etag and upload time, as R2 evaluates them.

use serde::Deserialize;

/// The conditions a read or write carries in `onlyIf`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Conditional {
    etag_matches: Option<Vec<Etag>>,
    etag_does_not_match: Option<Vec<Etag>>,
    /// Milliseconds since the epoch.
    uploaded_before: Option<i64>,
    /// Milliseconds since the epoch.
    uploaded_after: Option<i64>,
    #[serde(default)]
    seconds_granularity: bool,
}

/// An etag a condition names.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Etag {
    Strong { value: String },
    Weak { value: String },
    Wildcard,
}

impl Conditional {
    /// Whether the object with `etag`, uploaded at `uploaded` milliseconds
    /// since the epoch, meets the conditions; `current` is none when no
    /// object has the key.
    ///
    /// `uploadedBefore` also holds when `etagMatches` does, and
    /// `uploadedAfter` when `etagDoesNotMatch` does.
    pub(crate) fn holds(&self, current: Option<(&str, i64)>) -> bool {
        let Some((etag, uploaded)) = current else {
            return self.etag_matches.is_none() && self.uploaded_after.is_none();
        };
        let time = |milliseconds: i64| {
            if self.seconds_granularity {
                milliseconds.div_euclid(1000) * 1000
            } else {
                milliseconds
            }
        };
        let uploaded = time(uploaded);
        let matches = self
            .etag_matches
            .as_ref()
            .is_none_or(|etags| includes(etags, etag, false));
        let does_not_match = self
            .etag_does_not_match
            .as_ref()
            .is_none_or(|etags| !includes(etags, etag, true));
        let modified_since = self
            .uploaded_after
            .is_none_or(|after| time(after) < uploaded)
            || (self.etag_does_not_match.is_some() && does_not_match);
        let unmodified_since = self
            .uploaded_before
            .is_none_or(|before| uploaded < time(before))
            || (self.etag_matches.is_some() && matches);
        matches && does_not_match && modified_since && unmodified_since
    }
}

/// Whether `etags` names `etag`; weak etags count only when `weak` is set.
fn includes(etags: &[Etag], etag: &str, weak: bool) -> bool {
    etags.iter().any(|candidate| match candidate {
        Etag::Wildcard => true,
        Etag::Strong { value } => value == etag,
        Etag::Weak { value } => weak && value == etag,
    })
}

#[cfg(test)]
mod tests {
    use super::Conditional;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn conditional(json: &str) -> serde_json::Result<Conditional> {
        serde_json::from_str(json)
    }

    const OBJECT: Option<(&str, i64)> = Some(("abc", 10_500));

    #[test]
    fn matches_etags_strongly_and_excludes_them_weakly() -> TestResult {
        let strong = r#"[{"type": "strong", "value": "abc"}]"#;
        let weak = r#"[{"type": "weak", "value": "abc"}]"#;
        let wildcard = r#"[{"type": "wildcard"}]"#;

        assert!(conditional(&format!(r#"{{"etagMatches": {strong}}}"#))?.holds(OBJECT));
        assert!(!conditional(&format!(r#"{{"etagMatches": {weak}}}"#))?.holds(OBJECT));
        assert!(conditional(&format!(r#"{{"etagMatches": {wildcard}}}"#))?.holds(OBJECT));
        assert!(!conditional(&format!(r#"{{"etagDoesNotMatch": {weak}}}"#))?.holds(OBJECT));
        assert!(!conditional(&format!(r#"{{"etagDoesNotMatch": {wildcard}}}"#))?.holds(OBJECT));
        assert!(conditional(&format!(r#"{{"etagDoesNotMatch": {wildcard}}}"#))?.holds(None));
        assert!(!conditional(&format!(r#"{{"etagMatches": {wildcard}}}"#))?.holds(None));
        Ok(())
    }

    #[test]
    fn compares_upload_times_optionally_by_the_second() -> TestResult {
        assert!(conditional(r#"{"uploadedAfter": 10000}"#)?.holds(OBJECT));
        assert!(
            !conditional(r#"{"uploadedAfter": 10000, "secondsGranularity": true}"#)?.holds(OBJECT)
        );
        assert!(!conditional(r#"{"uploadedBefore": 10500}"#)?.holds(OBJECT));
        assert!(conditional(r#"{"uploadedBefore": 10501}"#)?.holds(OBJECT));
        assert!(!conditional(r#"{"uploadedAfter": 10000}"#)?.holds(None));
        assert!(conditional(r#"{"uploadedBefore": 0}"#)?.holds(None));
        Ok(())
    }

    #[test]
    fn lets_a_held_etag_condition_override_its_time_condition() -> TestResult {
        assert!(
            conditional(
                r#"{"uploadedBefore": 0, "etagMatches": [{"type": "strong", "value": "abc"}]}"#
            )?
            .holds(OBJECT)
        );
        assert!(
            conditional(r#"{"uploadedAfter": 99999, "etagDoesNotMatch": [{"type": "strong", "value": "x"}]}"#)?
                .holds(OBJECT)
        );
        Ok(())
    }
}
