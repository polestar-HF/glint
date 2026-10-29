//! Language codes.
//!
//! The app speaks BCP-47: `auto zh-Hans zh-Hant en ja ko fr de es ru pt it vi th ar`. Each
//! engine has its own spelling (Baidu says `jp` and `kor`, Youdao says `zh-CHS`, DeepL wants
//! `EN-US` as a target but `EN` as a source). The functions here translate between them and
//! return [`TranslateError::Config`] with a readable message when an engine cannot handle a
//! language.
//!
//! Input is lenient: case and `_` versus `-` do not matter, and region tags are folded onto
//! the 14 supported languages (`en-US` is `en`, `zh-TW` and `zh-HK` are `zh-Hant`, a bare `zh`
//! is `zh-Hans`).
//!
//! Only language coverage is checked. None of the engines document pair restrictions for these
//! 14 languages, so a pair is rejected only when it is meaningless: a target of `auto`.

use crate::{Result, TranslateError};

/// Whether a code is used as the source or as the target of a translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Source,
    Target,
}

/// A language the app can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Auto,
    ZhHans,
    ZhHant,
    En,
    Ja,
    Ko,
    Fr,
    De,
    Es,
    Ru,
    Pt,
    It,
    Vi,
    Th,
    Ar,
}

impl Lang {
    /// Parse a BCP-47-ish code. Returns `None` for anything outside the supported set.
    pub fn parse(code: &str) -> Option<Lang> {
        let norm = code.trim().replace('_', "-").to_ascii_lowercase();
        if norm.is_empty() {
            return None;
        }
        if norm == "auto" {
            return Some(Lang::Auto);
        }
        let mut tags = norm.split('-');
        let primary = tags.next().unwrap_or("");
        if primary == "zh" {
            // The script or region decides: `zh-Hant-HK` and `zh-TW` are Traditional; a bare
            // `zh`, `zh-CN`, `zh-SG` and `zh-Hans-CN` are Simplified.
            return Some(
                if tags.any(|t| matches!(t, "hant" | "tw" | "hk" | "mo" | "cht")) {
                    Lang::ZhHant
                } else {
                    Lang::ZhHans
                },
            );
        }
        Some(match primary {
            "en" => Lang::En,
            "ja" => Lang::Ja,
            "ko" => Lang::Ko,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "es" => Lang::Es,
            "ru" => Lang::Ru,
            "pt" => Lang::Pt,
            "it" => Lang::It,
            "vi" => Lang::Vi,
            "th" => Lang::Th,
            "ar" => Lang::Ar,
            _ => return None,
        })
    }

    /// The canonical BCP-47 code.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Auto => "auto",
            Lang::ZhHans => "zh-Hans",
            Lang::ZhHant => "zh-Hant",
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
            Lang::Fr => "fr",
            Lang::De => "de",
            Lang::Es => "es",
            Lang::Ru => "ru",
            Lang::Pt => "pt",
            Lang::It => "it",
            Lang::Vi => "vi",
            Lang::Th => "th",
            Lang::Ar => "ar",
        }
    }

    /// The English name, as used in LLM prompts.
    pub fn english_name(self) -> &'static str {
        match self {
            Lang::Auto => "the detected language",
            Lang::ZhHans => "Simplified Chinese",
            Lang::ZhHant => "Traditional Chinese",
            Lang::En => "English",
            Lang::Ja => "Japanese",
            Lang::Ko => "Korean",
            Lang::Fr => "French",
            Lang::De => "German",
            Lang::Es => "Spanish",
            Lang::Ru => "Russian",
            Lang::Pt => "Portuguese",
            Lang::It => "Italian",
            Lang::Vi => "Vietnamese",
            Lang::Th => "Thai",
            Lang::Ar => "Arabic",
        }
    }
}

/// True when the code means "detect the source language".
pub fn is_auto(code: &str) -> bool {
    matches!(Lang::parse(code), Some(Lang::Auto))
}

/// True when two codes name the same language (`en` and `en-US` do, `zh-Hans` and `zh-Hant`
/// do not). Unknown codes compare by their normalised text.
pub fn same_lang(a: &str, b: &str) -> bool {
    match (Lang::parse(a), Lang::parse(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim().eq_ignore_ascii_case(b.trim()),
    }
}

/// Like [`same_lang`], but for codes in an engine's own spelling, such as a language an engine
/// reported detecting (`EN`, `zh-CN`, `en2zh-CHS`'s `en`, Baidu's `cht`). Region tags are
/// ignored. Chinese counts as Traditional only when the code says so.
pub(crate) fn same_engine_lang(a: &str, b: &str) -> bool {
    fn key(code: &str) -> String {
        let c = code.trim().replace('_', "-").to_ascii_lowercase();
        let primary = c.split('-').next().unwrap_or("").to_string();
        if c == "cht"
            || (primary == "zh"
                && ["hant", "cht", "tw", "hk", "mo"]
                    .iter()
                    .any(|t| c.contains(t)))
        {
            return "zh-hant".into();
        }
        if primary == "zh" || c == "chs" {
            return "zh-hans".into();
        }
        primary
    }
    key(a) == key(b)
}

/// The language that should actually be the target. When the user chose an explicit source
/// that equals the target there is nothing to translate, so the secondary language steps in.
/// Returns the code unchanged otherwise (the engines flip automatically on detection when the
/// source is `auto`).
pub(crate) fn effective_target(source: &str, target: &str, secondary: Option<&str>) -> String {
    let usable_secondary = secondary
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_auto(s) && !same_lang(s, target));
    if !is_auto(source) && same_lang(source, target) {
        if let Some(sec) = usable_secondary {
            return sec.to_string();
        }
    }
    target.to_string()
}

/// A secondary language that may be used instead of `target`, if there is one.
pub(crate) fn usable_secondary<'a>(target: &str, secondary: Option<&'a str>) -> Option<&'a str> {
    secondary
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_auto(s) && !same_lang(s, target))
}

/// The language name to use in an LLM prompt. Unknown codes are passed through as written, so
/// a custom code such as `uk` still reaches the model.
pub(crate) fn llm_name(code: &str) -> String {
    match Lang::parse(code) {
        Some(l) => l.english_name().to_string(),
        None => code.trim().to_string(),
    }
}

fn resolve(engine: &str, code: &str, role: Role) -> Result<Lang> {
    let lang = Lang::parse(code).ok_or_else(|| {
        TranslateError::Config(format!(
            "{engine} does not support the language \"{}\"",
            code.trim()
        ))
    })?;
    if lang == Lang::Auto && role == Role::Target {
        return Err(TranslateError::Config(
            "A target language is required (\"auto\" only works as the source)".into(),
        ));
    }
    Ok(lang)
}

/// Google Translate (web endpoint and the v2 API share these codes).
pub fn google(code: &str, role: Role) -> Result<&'static str> {
    Ok(match resolve("Google", code, role)? {
        Lang::Auto => "auto",
        Lang::ZhHans => "zh-CN",
        Lang::ZhHant => "zh-TW",
        l => l.code(),
    })
}

/// Microsoft Translator. `auto` maps to an empty string, which means "omit `from`".
pub fn bing(code: &str, role: Role) -> Result<&'static str> {
    Ok(match resolve("Bing", code, role)? {
        Lang::Auto => "",
        l => l.code(),
    })
}

/// DeepL. Targets differ from sources: English and Portuguese need a variant as a target, and
/// Chinese needs a script. DeepL has no Vietnamese or Thai on older plans; the API reports that
/// itself, so the codes are passed through rather than rejected here.
pub fn deepl(code: &str, role: Role) -> Result<&'static str> {
    let lang = resolve("DeepL", code, role)?;
    Ok(match (lang, role) {
        (Lang::Auto, _) => "",
        (Lang::ZhHans, Role::Source) | (Lang::ZhHant, Role::Source) => "ZH",
        (Lang::ZhHans, Role::Target) => "ZH-HANS",
        (Lang::ZhHant, Role::Target) => "ZH-HANT",
        (Lang::En, Role::Source) => "EN",
        (Lang::En, Role::Target) => "EN-US",
        (Lang::Pt, Role::Source) => "PT",
        (Lang::Pt, Role::Target) => "PT-BR",
        (Lang::Ja, _) => "JA",
        (Lang::Ko, _) => "KO",
        (Lang::Fr, _) => "FR",
        (Lang::De, _) => "DE",
        (Lang::Es, _) => "ES",
        (Lang::Ru, _) => "RU",
        (Lang::It, _) => "IT",
        (Lang::Vi, _) => "VI",
        (Lang::Th, _) => "TH",
        (Lang::Ar, _) => "AR",
    })
}

/// Youdao openapi.
pub fn youdao(code: &str, role: Role) -> Result<&'static str> {
    Ok(match resolve("Youdao", code, role)? {
        Lang::Auto => "auto",
        Lang::ZhHans => "zh-CHS",
        Lang::ZhHant => "zh-CHT",
        l => l.code(),
    })
}

/// Baidu fanyi.
pub fn baidu(code: &str, role: Role) -> Result<&'static str> {
    Ok(match resolve("Baidu", code, role)? {
        Lang::Auto => "auto",
        Lang::ZhHans => "zh",
        Lang::ZhHant => "cht",
        Lang::En => "en",
        Lang::Ja => "jp",
        Lang::Ko => "kor",
        Lang::Fr => "fra",
        Lang::De => "de",
        Lang::Es => "spa",
        Lang::Ru => "ru",
        Lang::Pt => "pt",
        Lang::It => "it",
        Lang::Vi => "vie",
        Lang::Th => "th",
        Lang::Ar => "ara",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&str; 14] = [
        "zh-Hans", "zh-Hant", "en", "ja", "ko", "fr", "de", "es", "ru", "pt", "it", "vi", "th",
        "ar",
    ];

    #[test]
    fn parse_is_lenient_and_folds_regions() {
        assert_eq!(Lang::parse("EN-us"), Some(Lang::En));
        assert_eq!(Lang::parse("en_GB"), Some(Lang::En));
        assert_eq!(Lang::parse("zh"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh_tw"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-Hant-HK"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-Hans-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh-CHT"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("auto-x"), None);
        assert_eq!(Lang::parse("pt-BR"), Some(Lang::Pt));
        assert_eq!(Lang::parse(" AUTO "), Some(Lang::Auto));
        assert_eq!(Lang::parse(""), None);
        assert_eq!(Lang::parse("xx"), None);
        assert_eq!(Lang::parse("klingon"), None);
    }

    #[test]
    fn canonical_codes_round_trip() {
        for c in ALL.iter().chain(["auto"].iter()) {
            assert_eq!(Lang::parse(c).map(Lang::code), Some(*c));
        }
    }

    #[test]
    fn every_engine_covers_every_supported_language() {
        for c in ALL {
            for role in [Role::Source, Role::Target] {
                for f in [google, bing, deepl, youdao, baidu] {
                    let code = f(c, role).unwrap_or_else(|e| panic!("{c}: {e}"));
                    assert!(!code.is_empty(), "{c} {role:?} maps to an empty code");
                }
            }
        }
    }

    #[test]
    fn engine_specific_spellings() {
        assert_eq!(google("zh-Hans", Role::Target).ok(), Some("zh-CN"));
        assert_eq!(google("zh-Hant", Role::Target).ok(), Some("zh-TW"));
        assert_eq!(google("auto", Role::Source).ok(), Some("auto"));
        assert_eq!(bing("zh-Hant", Role::Target).ok(), Some("zh-Hant"));
        assert_eq!(bing("auto", Role::Source).ok(), Some(""));
        assert_eq!(deepl("en", Role::Source).ok(), Some("EN"));
        assert_eq!(deepl("en", Role::Target).ok(), Some("EN-US"));
        assert_eq!(deepl("pt", Role::Target).ok(), Some("PT-BR"));
        assert_eq!(deepl("zh-Hant", Role::Source).ok(), Some("ZH"));
        assert_eq!(deepl("zh-Hant", Role::Target).ok(), Some("ZH-HANT"));
        assert_eq!(youdao("zh-Hans", Role::Source).ok(), Some("zh-CHS"));
        assert_eq!(youdao("zh-Hant", Role::Target).ok(), Some("zh-CHT"));
        assert_eq!(baidu("ja", Role::Target).ok(), Some("jp"));
        assert_eq!(baidu("ko", Role::Target).ok(), Some("kor"));
        assert_eq!(baidu("fr", Role::Target).ok(), Some("fra"));
        assert_eq!(baidu("es", Role::Target).ok(), Some("spa"));
        assert_eq!(baidu("vi", Role::Target).ok(), Some("vie"));
        assert_eq!(baidu("ar", Role::Target).ok(), Some("ara"));
        assert_eq!(baidu("zh-Hant", Role::Target).ok(), Some("cht"));
    }

    #[test]
    fn unsupported_codes_and_auto_target_are_clear_errors() {
        let e = google("xx", Role::Target).unwrap_err();
        assert!(
            matches!(e, TranslateError::Config(ref m) if m.contains("Google") && m.contains("xx"))
        );
        let e = baidu("auto", Role::Target).unwrap_err();
        assert!(matches!(e, TranslateError::Config(ref m) if m.contains("target language")));
        assert!(deepl("tlh", Role::Source).is_err());
    }

    #[test]
    fn same_lang_distinguishes_chinese_scripts() {
        assert!(same_lang("en", "en-US"));
        assert!(same_lang("zh", "zh-CN"));
        assert!(!same_lang("zh-Hans", "zh-Hant"));
        assert!(!same_lang("en", "ja"));
        assert!(same_lang("uk", "UK"));
    }

    #[test]
    fn same_engine_lang_compares_in_engine_spelling() {
        assert!(same_engine_lang("en", "EN-US"));
        assert!(same_engine_lang("EN", "en"));
        assert!(same_engine_lang("zh-CN", "ZH-HANS"));
        assert!(same_engine_lang("zh", "zh-CHS"));
        assert!(same_engine_lang("zh-TW", "zh-Hant"));
        assert!(same_engine_lang("cht", "zh-CHT"));
        assert!(!same_engine_lang("zh-CN", "zh-TW"));
        assert!(!same_engine_lang("zh", "cht"));
        assert!(!same_engine_lang("en", "jp"));
        assert!(same_engine_lang("PT", "PT-BR"));
    }

    #[test]
    fn explicit_source_equal_to_target_falls_back_to_secondary() {
        assert_eq!(effective_target("en", "en", Some("zh-Hans")), "zh-Hans");
        assert_eq!(effective_target("en", "en-US", Some("zh-Hans")), "zh-Hans");
        assert_eq!(effective_target("en", "en", None), "en");
        assert_eq!(effective_target("en", "en", Some("en")), "en");
        assert_eq!(effective_target("auto", "en", Some("zh-Hans")), "en");
        assert_eq!(effective_target("ja", "en", Some("zh-Hans")), "en");
    }

    #[test]
    fn llm_names() {
        assert_eq!(llm_name("zh-Hans"), "Simplified Chinese");
        assert_eq!(llm_name("zh-TW"), "Traditional Chinese");
        assert_eq!(llm_name("uk"), "uk");
    }
}
