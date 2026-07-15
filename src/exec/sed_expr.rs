use regex::Regex;

#[derive(Debug)]
pub struct Substitution {
    pub regex: Regex,
    pub replacement: String,
    pub global: bool,
}

impl Substitution {
    pub fn apply(&self, input: &str) -> Option<String> {
        let out = if self.global {
            self.regex.replace_all(input, self.replacement.as_str())
        } else {
            self.regex.replace(input, self.replacement.as_str())
        };
        if out == input {
            None
        } else {
            Some(out.into_owned())
        }
    }
}

pub fn parse(expr: &str) -> Result<Substitution, String> {
    let mut chars = expr.chars();
    match chars.next() {
        Some('s') => {}
        Some('y') => return Err("y/// transliteration is not supported".into()),
        _ => return Err(format!("'{expr}' is not an s/// expression")),
    }
    let delim = chars.next().ok_or("missing delimiter")?;
    if !delim.is_ascii_punctuation() || delim == '\\' {
        return Err(format!("unsupported delimiter '{delim}'"));
    }
    if matches!(delim, '(' | '[' | '{' | '<') {
        return Err("bracket-pair delimiters are not supported".into());
    }

    let rest: Vec<char> = chars.collect();
    let sections = split_sections(&rest, delim)?;
    let [pattern_raw, replacement_raw, flags] = sections;

    let mut global = false;
    let mut case_insensitive = false;
    for f in flags.chars() {
        match f {
            'g' => global = true,
            'i' => case_insensitive = true,
            other => return Err(format!("unsupported s/// flag '{other}'")),
        }
    }

    let pattern = unescape_delim(&pattern_raw, delim, true);
    if pattern.is_empty() {
        return Err("empty pattern (perl reuses the last regex — unsupported)".into());
    }
    let replacement = translate_replacement(&unescape_delim(&replacement_raw, delim, false))?;

    let full = if case_insensitive {
        format!("(?i){pattern}")
    } else {
        pattern
    };
    let regex =
        Regex::new(&full).map_err(|e| format!("pattern not supported by the regex engine: {e}"))?;

    Ok(Substitution {
        regex,
        replacement,
        global,
    })
}

fn split_sections(rest: &[char], delim: char) -> Result<[String; 3], String> {
    let mut sections = vec![String::new()];
    let mut i = 0;
    while i < rest.len() {
        let c = rest[i];
        if c == '\\' && i + 1 < rest.len() {
            let cur = sections.last_mut().expect("non-empty");
            cur.push(c);
            cur.push(rest[i + 1]);
            i += 2;
            continue;
        }
        if c == delim {
            sections.push(String::new());
            i += 1;
            continue;
        }
        sections.last_mut().expect("non-empty").push(c);
        i += 1;
    }
    match <[String; 3]>::try_from(sections) {
        Ok(three) => Ok(three),
        Err(parts) => Err(format!(
            "expected s{delim}PATTERN{delim}REPLACEMENT{delim}[flags], found {} section(s)",
            parts.len()
        )),
    }
}

fn unescape_delim(section: &str, delim: char, is_regex: bool) -> String {
    const REGEX_SPECIAL: &str = r".^$|()[]{}*+?\";
    let mut out = String::new();
    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&delim) {
            let d = chars.next().expect("peeked");
            if is_regex && REGEX_SPECIAL.contains(d) {
                out.push('\\');
            }
            out.push(d);
        } else {
            out.push(c);
        }
    }
    out
}

fn translate_replacement(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => out.push_str(&format!("${{{d}}}")),
                Some(e @ ('u' | 'l' | 'U' | 'L' | 'E')) => {
                    return Err(format!("case-conversion escape '\\{e}' is not supported"));
                }
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            '$' => match chars.peek() {
                Some('0'..='9') => {
                    let d = chars.next().expect("peeked");
                    out.push_str(&format!("${{{d}}}"));
                }
                Some('{') => out.push('$'),
                _ => out.push_str("$$"),
            },
            other => out.push(other),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(expr: &str) -> Substitution {
        parse(expr).unwrap()
    }

    #[test]
    fn basic_substitution() {
        let s = sub("s/foo/bar/");
        assert_eq!(s.apply("foo.txt").as_deref(), Some("bar.txt"));
        assert_eq!(s.apply("nope"), None);
    }

    #[test]
    fn first_match_vs_global() {
        assert_eq!(sub("s/a/b/").apply("aaa").as_deref(), Some("baa"));
        assert_eq!(sub("s/a/b/g").apply("aaa").as_deref(), Some("bbb"));
    }

    #[test]
    fn case_insensitive_flag() {
        assert_eq!(sub("s/FOO/bar/i").apply("foo").as_deref(), Some("bar"));
    }

    #[test]
    fn alternative_delimiters() {
        assert_eq!(
            sub("s#/old/#/new/#").apply("/old/x").as_deref(),
            Some("/new/x")
        );
        assert_eq!(sub("s,a,b,").apply("a").as_deref(), Some("b"));
        assert_eq!(sub("s|a|b|g").apply("aa").as_deref(), Some("bb"));
    }

    #[test]
    fn escaped_delimiter() {
        assert_eq!(sub(r"s/a\/b/x/").apply("a/b").as_deref(), Some("x"));
        assert_eq!(sub(r"s.a\.b.x.").apply("a.b").as_deref(), Some("x"));
        assert_eq!(sub(r"s.a\.b.x.").apply("aXb"), None);
    }

    #[test]
    fn backreferences_both_spellings() {
        assert_eq!(
            sub(r"s/(\d+)-(\d+)/\2-\1/").apply("12-34").as_deref(),
            Some("34-12")
        );
        assert_eq!(
            sub(r"s/(\w+)\.txt/$1.md/").apply("note.txt").as_deref(),
            Some("note.md")
        );
    }

    #[test]
    fn literal_dollar_is_escaped() {
        assert_eq!(sub(r"s/x/$/").apply("x").as_deref(), Some("$"));
    }

    #[test]
    fn rejections() {
        for bad in [
            "y/a/b/",
            "s/a/b",
            "s/a/b/e",
            "s/a/b/x",
            "s//b/",
            "s{a}{b}",
            r"s/a/\Ub/",
            "plain",
            "s/a(?<=b)/c/",
        ] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn unicode_input() {
        assert_eq!(sub("s/ü/ue/g").apply("übung").as_deref(), Some("uebung"));
    }
}
