//! The text setter's grammar pass, Wrath's `|1`..`|4` escapes, as the FontString setter
//! (`0x483910`, `CSimpleRender.cpp`) runs it before storing the text when its flag is set: the pass
//! `0x482110` walks the string by character, skips `||`, and on `|` and a number (`0x76f0d0`)
//! rewrites the whole string through that escape's resolver, then walks the result again. The
//! stored, drawn and `GetText` string is the resolved one.
//!
//! `|4` is the plural (`0x481fc0`), implemented here; `|1`, `|2` and `|3` (`0x481a00`, `0x481c80`,
//! `0x481e90`) are the locale grammar escapes (articles, elision and declension) that no enUS
//! string uses, and pass through literally until a locale needs them.

use std::borrow::Cow;

/// The plural rule set by `0x817d00` into `0xd3f7a0`, read by the form picker `0x817d10`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PluralRule {
    /// Rule 0: one form for 1, another for everything else (English, German, Spanish).
    #[default]
    OneOther,
    /// Rule 1: one form for 0 and 1, another above (French).
    ZeroOneOther,
    /// Rule 2: the three Russian forms by the last two digits.
    Slavic,
}

/// This client's rule: WrathPlus is enUS-only (`GetLocale`), rule 0. Which locale sets rule 1 or 2
/// (French and Russian by their shapes) was not traced.
pub const CLIENT_RULE: PluralRule = PluralRule::OneOther;

impl PluralRule {
    /// `0x817d10`: the 0-based form a count takes.
    pub fn form(self, n: i32) -> u32 {
        match self {
            PluralRule::OneOther => u32::from(n != 1),
            PluralRule::ZeroOneOther => u32::from(n > 1),
            PluralRule::Slavic => {
                // C `%` and the unsigned compares, so a wrapped negative count reads as they do.
                if ((n % 100) as u32).wrapping_sub(11) < 4 {
                    2
                } else if n % 10 != 1 {
                    1 + u32::from(((n % 10) as u32).wrapping_sub(2) > 2)
                } else {
                    0
                }
            }
        }
    }
}

/// The pass's input cap (`0x482129`): a longer string is cut to its first `0x1ffd` bytes, here
/// backed off to a character boundary.
const CAP: usize = 0x2000;
const CUT: usize = 0x1ffd;

/// `0x482110`: resolve the grammar escapes in a string bound for a FontString.
pub fn resolve(text: &str, rule: PluralRule) -> Cow<'_, str> {
    let mut s: Cow<'_, str> = if text.len() > CAP {
        let mut cut = CUT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        Cow::Borrowed(&text[..cut])
    } else {
        Cow::Borrowed(text)
    };
    // Each `|4` hit rewrites every `|4` in the string, so one rewrite and a fresh walk resolve
    // them all; a walk that finds none is done. A hit the rewrite leaves standing (`|04`, whose
    // number reads 4 but whose bytes are not `|4`) would loop the reference forever; here it
    // stands.
    while has_plural(&s) {
        let next = plural(s.as_bytes(), rule);
        if next == s.as_bytes() {
            break;
        }
        // Delimiters are ASCII, so every copied run ends on a character boundary.
        s = Cow::Owned(String::from_utf8(next).expect("cut at ASCII delimiters"));
    }
    s
}

/// [`resolve`] under [`CLIENT_RULE`] for a text sink that owns its string: the same allocation
/// when nothing resolves.
pub fn resolve_owned(text: String) -> String {
    match resolve(&text, CLIENT_RULE) {
        Cow::Borrowed(same) if same.len() == text.len() => text,
        changed => changed.into_owned(),
    }
}

/// The walk of `0x482110`: whether a `|` and a number reading 4 stands outside a `||`.
fn has_plural(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'|' {
            match b.get(i + 1) {
                Some(b'|') => i += 1,
                Some(d) if d.is_ascii_digit() => {
                    let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit());
                    let n = digits.fold(0u32, |n, &c| {
                        n.wrapping_mul(10).wrapping_add(u32::from(c - b'0'))
                    });
                    if n == 4 {
                        return true;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    false
}

/// `0x481fc0`: every `|4form0:form1:…;` becomes the form the last number before it picks. The
/// number is the last run of ASCII digits anywhere earlier, outside the escapes themselves (a
/// colour's hex digits count), 0 when there is none. A `|` and any other byte, or `|4` and a digit,
/// copy through as a pair. The picked form loses its leading spaces; a missing form is empty; an
/// unterminated escape ends the string.
fn plural(s: &[u8], rule: PluralRule) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut n: i32 = 0;
    let mut in_run = false;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c.is_ascii_digit() {
            let d = i32::from(c - b'0');
            n = if in_run {
                n.wrapping_mul(10).wrapping_add(d)
            } else {
                d
            };
            in_run = true;
        } else {
            in_run = false;
        }
        if c != b'|' {
            out.push(c);
            i += 1;
            continue;
        }
        let next = s.get(i + 1).copied();
        if next != Some(b'4') || s.get(i + 2).is_some_and(u8::is_ascii_digit) {
            out.push(b'|');
            // The pair's second byte is copied unread; a trailing `|` ends the string (the
            // reference reads past its terminator).
            let Some(x) = next else { break };
            out.push(x);
            i += 2;
            continue;
        }
        let mut p = i + 2;
        let want = rule.form(n);
        let mut k = 0;
        if want != 0 {
            'skip: while p < s.len() {
                while s[p] != b':' && s[p] != b';' {
                    p += 1;
                    if p >= s.len() {
                        break 'skip;
                    }
                }
                if s[p] == b';' {
                    break;
                }
                k += 1;
                p += 1;
                if k >= want {
                    break;
                }
            }
        }
        while p < s.len() && s[p] == b' ' {
            p += 1;
        }
        if p >= s.len() {
            break;
        }
        while s[p] != b':' && s[p] != b';' {
            out.push(s[p]);
            p += 1;
            if p >= s.len() {
                return out;
            }
        }
        while s[p] != b';' {
            p += 1;
            if p >= s.len() {
                return out;
            }
        }
        i = p + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en(s: &str) -> String {
        resolve(s, PluralRule::OneOther).into_owned()
    }

    #[test]
    fn the_last_number_before_the_escape_picks_the_form() {
        // LEVEL_UP_CHAR_POINTS after format.
        assert_eq!(
            en("You gain 1 talent |4point:points;."),
            "You gain 1 talent point."
        );
        assert_eq!(
            en("You gain 3 talent |4point:points;."),
            "You gain 3 talent points."
        );
        // No number yet: 0, plural in English.
        assert_eq!(en("|4point:points;"), "points");
        // The last run counts, and each escape reads it afresh.
        assert_eq!(
            en("10 of 1 |4item:items; and |4coin:coins;"),
            "10 of 1 item and coin"
        );
        // A colour's hex digits are a run like any other.
        assert_eq!(en("2 |cff000001|r|4a:b;"), "2 |cff000001|ra");
    }

    #[test]
    fn forms_trim_leading_spaces_and_a_missing_form_is_empty() {
        assert_eq!(en("2 |4 one: many;"), "2 many");
        assert_eq!(en("2 |4one;"), "2 ");
        assert_eq!(en("1 |4one:two:three;!"), "1 one!");
    }

    #[test]
    fn escaped_bars_other_escapes_and_unterminated_forms() {
        assert_eq!(en("2 ||4a:b;"), "2 ||4a:b;");
        assert_eq!(en("2 |45 |cffffffff"), "2 |45 |cffffffff");
        // An unterminated escape ends the string where its form does.
        assert_eq!(en("2 |4a:b"), "2 b");
        assert_eq!(en("1 |4a"), "1 a");
        assert_eq!(en("1 |4"), "1 ");
        // `|1`..`|3` are not resolved here.
        assert_eq!(en("|1a;b;"), "|1a;b;");
    }

    #[test]
    fn the_three_rules_pick_as_0x817d10() {
        let fr = PluralRule::ZeroOneOther;
        assert_eq!((fr.form(0), fr.form(1), fr.form(2)), (0, 0, 1));
        let ru = PluralRule::Slavic;
        let picks: Vec<u32> = [1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111]
            .iter()
            .map(|&n| ru.form(n))
            .collect();
        assert_eq!(picks, vec![0, 1, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2]);
        assert_eq!(resolve("5 |4a:b:c;", ru), "5 c");
    }

    #[test]
    fn a_long_string_is_cut_before_the_pass() {
        let long = "x".repeat(0x2001);
        assert_eq!(resolve(&long, PluralRule::OneOther).len(), 0x1ffd);
        let short = "y".repeat(0x2000);
        assert_eq!(resolve(&short, PluralRule::OneOther).len(), 0x2000);
    }
}
