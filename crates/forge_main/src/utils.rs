use std::time::Duration;

/// Formats a chrono DateTime as a human-readable relative time string (e.g., "5
/// minutes ago").
pub fn humanize_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    let duration = chrono::Utc::now().signed_duration_since(dt);
    let duration = Duration::from_secs((duration.num_minutes() * 60).max(0) as u64);
    if duration.is_zero() {
        "now".to_string()
    } else {
        format!("{} ago", humantime::format_duration(duration))
    }
}

/// Humanizes a number to a readable format with B/M/k suffixes.
///
/// # Arguments
/// * `n` - The number to humanize
///
/// # Examples
/// ```ignore
/// assert_eq!(humanize_number(1500), "1.5k");
/// assert_eq!(humanize_number(1500000), "1.5M");
/// assert_eq!(humanize_number(1500000000), "1.5B");
/// assert_eq!(humanize_number(500), "500");
/// ```
pub fn humanize_number(n: usize) -> String {
    match n {
        n if n >= 1_000_000_000 => {
            let val = n as f64 / 1_000_000_000.0;
            if (val * 10.0).round() % 10.0 == 0.0 {
                format!("{:.0}B", val)
            } else {
                format!("{:.1}B", val)
            }
        }
        n if n >= 1_000_000 => {
            let val = n as f64 / 1_000_000.0;
            if (val * 10.0).round() % 10.0 == 0.0 {
                format!("{:.0}M", val)
            } else {
                format!("{:.1}M", val)
            }
        }
        n if n >= 1_000 => {
            let val = n as f64 / 1_000.0;
            if (val * 10.0).round() % 10.0 == 0.0 {
                format!("{:.0}k", val)
            } else {
                format!("{:.1}k", val)
            }
        }
        _ => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_humanize_number_billions() {
        let actual = humanize_number(1_500_000_000);
        let expected = "1.5B";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_humanize_number_millions() {
        let actual = humanize_number(2_300_000);
        let expected = "2.3M";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_humanize_number_thousands() {
        let actual = humanize_number(4_500);
        let expected = "4.5k";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_humanize_number_whole_thousands() {
        let actual = humanize_number(15_000);
        let expected = "15k";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_humanize_number_small() {
        let actual = humanize_number(999);
        let expected = "999";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_humanize_number_zero() {
        let actual = humanize_number(0);
        let expected = "0";
        assert_eq!(actual, expected);
    }
}
