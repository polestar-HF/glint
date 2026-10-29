//! Language tag matching for `Windows.Media.Ocr` (kept platform-neutral so it is unit-tested
//! everywhere).
//!
//! Windows OCR language packs have full tags such as `en-US`, `zh-Hans-CN`, `zh-Hant-TW`,
//! `ja`, `ko`, while the app passes short hints (`en`, `zh`, `zh-Hans`, `ja`).

/// The installed language tag that best matches `hint`, or `None` for `auto` / empty hints and
/// when nothing matches.
///
/// Rules, in order: exact match; same primary language with the right Chinese script
/// (`zh`, `zh-CN`, `zh-Hans` want Simplified; `zh-TW`, `zh-HK`, `zh-Hant` Traditional); the
/// language's "home" region (`en-US`, `ja-JP`, ...); otherwise the first of that language.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn pick_language_tag(hint: &str, available: &[String]) -> Option<String> {
    let hint = hint.trim().replace('_', "-").to_ascii_lowercase();
    if hint.is_empty() || hint == "auto" {
        return None;
    }
    let primary = |t: &str| t.split('-').next().unwrap_or("").to_ascii_lowercase();
    if let Some(exact) = available.iter().find(|t| t.eq_ignore_ascii_case(&hint)) {
        return Some(exact.clone());
    }
    let want = primary(&hint);
    let mut same: Vec<&String> = available.iter().filter(|t| primary(t) == want).collect();
    if same.is_empty() {
        return None;
    }
    if want == "zh" {
        let traditional =
            hint.contains("hant") || ["-tw", "-hk", "-mo"].iter().any(|r| hint.ends_with(r));
        let script = if traditional { "hant" } else { "hans" };
        let by_script: Vec<&String> = same
            .iter()
            .copied()
            .filter(|t| {
                let l = t.to_ascii_lowercase();
                l.contains(script)
                    || (script == "hans" && (l.ends_with("-cn") || l.ends_with("-sg")))
                    || (script == "hant"
                        && (l.ends_with("-tw") || l.ends_with("-hk") || l.ends_with("-mo")))
            })
            .collect();
        if !by_script.is_empty() {
            same = by_script;
        }
    }
    // A hint with a script or region ("zh-hans") prefers the tag that extends it ("zh-Hans-CN").
    if hint.contains('-') {
        if let Some(t) = same
            .iter()
            .find(|t| t.to_ascii_lowercase().starts_with(&format!("{hint}-")))
        {
            return Some((*t).clone());
        }
    }
    let home = format!("{want}-{}", want.to_ascii_uppercase());
    let special = match want.as_str() {
        "en" => "en-us",
        "ja" => "ja-jp",
        "ko" => "ko-kr",
        _ => "",
    };
    same.iter()
        .find(|t| {
            let l = t.to_ascii_lowercase();
            l == special || l == home.to_ascii_lowercase()
        })
        .or_else(|| same.first())
        .map(|t| (*t).clone())
}

#[cfg(test)]
mod tests {
    use super::pick_language_tag as pick;

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn auto_and_empty_pick_nothing() {
        let a = tags(&["en-US"]);
        assert_eq!(pick("auto", &a), None);
        assert_eq!(pick("", &a), None);
        assert_eq!(pick("  AUTO ", &a), None);
    }

    #[test]
    fn exact_and_case_insensitive() {
        let a = tags(&["en-US", "de-DE"]);
        assert_eq!(pick("en-us", &a).as_deref(), Some("en-US"));
        assert_eq!(pick("de_DE", &a).as_deref(), Some("de-DE"));
    }

    #[test]
    fn short_hints_find_regional_packs() {
        let a = tags(&[
            "en-GB",
            "en-US",
            "ja",
            "ko",
            "fr-FR",
            "zh-Hans-CN",
            "zh-Hant-TW",
        ]);
        assert_eq!(pick("en", &a).as_deref(), Some("en-US"));
        assert_eq!(pick("fr", &a).as_deref(), Some("fr-FR"));
        assert_eq!(pick("ja", &a).as_deref(), Some("ja"));
        assert_eq!(pick("ko", &a).as_deref(), Some("ko"));
        assert_eq!(pick("en-gb", &a).as_deref(), Some("en-GB"));
    }

    #[test]
    fn chinese_scripts() {
        let a = tags(&["zh-Hant-TW", "zh-Hans-CN"]);
        assert_eq!(pick("zh", &a).as_deref(), Some("zh-Hans-CN"));
        assert_eq!(pick("zh-CN", &a).as_deref(), Some("zh-Hans-CN"));
        assert_eq!(pick("zh-Hans", &a).as_deref(), Some("zh-Hans-CN"));
        assert_eq!(pick("zh-TW", &a).as_deref(), Some("zh-Hant-TW"));
        assert_eq!(pick("zh-Hant", &a).as_deref(), Some("zh-Hant-TW"));
        // Only Traditional installed: still better than nothing for a "zh" hint.
        assert_eq!(
            pick("zh", &tags(&["zh-Hant-TW"])).as_deref(),
            Some("zh-Hant-TW")
        );
    }

    #[test]
    fn missing_language_is_none() {
        assert_eq!(pick("ru", &tags(&["en-US", "ja"])), None);
        assert_eq!(pick("en", &[]), None);
    }
}
