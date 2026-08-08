use chrono::{Local, SecondsFormat};

pub fn event_timestamp(observed_at: Option<&str>) -> String {
    observed_at
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Local::now().to_rfc3339_opts(SecondsFormat::Secs, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_server_observation_timestamp() {
        assert_eq!(
            event_timestamp(Some(" 2026-08-08T14:06:01.000Z ")),
            "2026-08-08T14:06:01.000Z"
        );
    }

    #[test]
    fn supplies_timestamp_when_server_omits_it() {
        assert!(!event_timestamp(None).is_empty());
        assert!(!event_timestamp(Some("  ")).is_empty());
    }
}
