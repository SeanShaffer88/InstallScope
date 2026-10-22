//! Stable numeric constraint subsets. Never reinterpret unsupported syntax as a match.
use anyhow::{bail, Result};
use semver::Version;
fn numeric(text: &str) -> Result<(Version, usize)> {
    let parts: Vec<_> = text.split('.').collect();
    if parts.is_empty()
        || parts.len() > 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        bail!("Only stable numeric versions with 1–3 components are assessed; this declaration remains unverified");
    }
    let mut v = [0; 3];
    for (i, p) in parts.iter().enumerate() {
        v[i] = p.parse::<u64>()?;
    }
    Ok((Version::new(v[0], v[1], v[2]), parts.len()))
}
fn node_numeric(text: &str) -> Result<(Version, usize)> {
    if text.split('.').any(|p| p.len() > 1 && p.starts_with('0')) {
        bail!("Leading-zero Node versions are not assessed");
    }
    numeric(text)
}
fn next(v: &Version, count: usize) -> Result<Version> {
    let add = |x: u64| {
        x.checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Version range overflow"))
    };
    Ok(match count {
        1 => Version::new(add(v.major)?, 0, 0),
        2 => Version::new(v.major, add(v.minor)?, 0),
        _ => Version::new(v.major, v.minor, add(v.patch)?),
    })
}
fn node_token(token: &str, actual: &Version) -> Result<bool> {
    if matches!(token, "*" | "x" | "X") {
        return Ok(true);
    }
    let (op, value) = [">=", "<=", ">", "<", "=", "^", "~"]
        .iter()
        .find_map(|op| token.strip_prefix(op).map(|v| (*op, v)))
        .unwrap_or(("", token));
    let value = value.strip_prefix('v').unwrap_or(value);
    let parts: Vec<_> = value.split('.').collect();
    let wildcard = parts.iter().position(|p| matches!(*p, "*" | "x" | "X"));
    if let Some(at) = wildcard {
        if !op.is_empty()
            || parts.len() > 3
            || parts[at..].iter().any(|p| !matches!(*p, "*" | "x" | "X"))
        {
            bail!("Unsupported wildcard comparison");
        }
        if at == 0 {
            return Ok(true);
        }
        let (low, n) = node_numeric(&parts[..at].join("."))?;
        return Ok(actual >= &low && actual < &next(&low, n)?);
    }
    let (low, count) = node_numeric(value)?;
    Ok(match op {
        ">=" => actual >= &low,
        "<" => actual < &low,
        ">" if count < 3 => actual >= &next(&low, count)?,
        ">" => actual > &low,
        "<=" if count < 3 => actual < &next(&low, count)?,
        "<=" => actual <= &low,
        "~" => actual >= &low && actual < &next(&low, if count == 1 { 1 } else { 2 })?,
        "^" => {
            let upper = if low.major > 0 || count == 1 {
                next(&low, 1)?
            } else if low.minor > 0 || count == 2 {
                next(&low, 2)?
            } else {
                next(&low, 3)?
            };
            actual >= &low && actual < &upper
        }
        "" | "=" if count < 3 => actual >= &low && actual < &next(&low, count)?,
        "" | "=" => actual == &low,
        _ => bail!("Unsupported Node engine operator"),
    })
}
fn node(text: &str, actual: &Version) -> Result<bool> {
    let alternatives: Vec<_> = text.split("||").collect();
    if alternatives.len() > 8 {
        bail!("More than 8 alternative ranges");
    }
    let mut any = false;
    // Validate every branch, even if an earlier branch matches.
    for branch in alternatives {
        let tokens: Vec<_> = branch.split_whitespace().collect();
        if tokens.is_empty() || tokens.len() > 16 {
            bail!("Empty or excessive Node engine range");
        }
        if tokens.len() == 3 && tokens[1] == "-" {
            let (lo, _) = node_numeric(tokens[0].trim_start_matches('v'))?;
            let (hi, n) = node_numeric(tokens[2].trim_start_matches('v'))?;
            any |= actual >= &lo
                && if n < 3 {
                    actual < &next(&hi, n)?
                } else {
                    actual <= &hi
                };
            continue;
        }
        let mut all = true;
        let mut i = 0;
        while i < tokens.len() {
            let mut token = tokens[i].to_owned();
            if matches!(token.as_str(), ">=" | "<=" | ">" | "<" | "=" | "^" | "~") {
                i += 1;
                let Some(value) = tokens.get(i) else {
                    bail!("Missing comparator version");
                };
                token.push_str(value);
            }
            all &= node_token(&token, actual)?;
            i += 1;
        }
        any |= all;
    }
    Ok(any)
}
fn python(text: &str, actual: &Version) -> Result<bool> {
    let tokens: Vec<_> = text.split(',').map(str::trim).collect();
    if tokens.is_empty() || tokens.len() > 16 {
        bail!("Excess Python comparisons");
    }
    let mut all = true;
    for token in tokens {
        let Some((op, value)) = [">=", "<=", "==", "!=", "~=", ">", "<"]
            .iter()
            .find_map(|op| token.strip_prefix(op).map(|v| (*op, v.trim())))
        else {
            bail!("Unsupported Python specifier; no PEP 440 compatibility claim is made");
        };
        if let Some(prefix) = value.strip_suffix(".*") {
            if !matches!(op, "==" | "!=") {
                bail!("Wildcard requires == or !=");
            }
            let (lo, n) = numeric(prefix)?;
            let matches = actual >= &lo && actual < &next(&lo, n)?;
            all &= if op == "==" { matches } else { !matches };
            continue;
        }
        let (v, n) = numeric(value)?;
        all &= match op {
            ">=" => actual >= &v,
            "<=" => actual <= &v,
            ">" => actual > &v,
            "<" => actual < &v,
            "==" => actual == &v,
            "!=" => actual != &v,
            "~=" if n >= 2 => actual >= &v && actual < &next(&v, n - 1)?,
            _ => bail!("Unsupported compatible-release form"),
        };
    }
    Ok(all)
}
pub(super) fn matches(tool: &str, text: &str, actual: &Version) -> Result<bool> {
    if text.len() > 256 || text.chars().any(char::is_control) || !actual.pre.is_empty() {
        bail!("Unverified prerelease or excessive declaration");
    }
    match tool {
        "Node.js" => node(text, actual),
        "Python" => python(text, actual),
        "Rust minimum" => {
            let (v, _) = numeric(text)?;
            Ok(actual >= &v)
        }
        "Node.js pin" | "Python pin" => {
            let (v, n) = numeric(if tool == "Node.js pin" {
                text.strip_prefix('v').unwrap_or(text)
            } else {
                text
            })?;
            if n != 3 {
                bail!(
                    "Partial/alias pins require a version manager; exact selection remains unknown"
                );
            }
            Ok(actual == &v)
        }
        _ => bail!("No version comparison for this declaration"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }
    #[test]
    fn node_major_comparators_and_caret_zero() {
        for (range, version, yes) in [
            (">=18 <22", "21.9.0", true),
            (">=18 <22", "22.0.0", false),
            ("<=18", "18.20.0", true),
            (">18", "18.20.0", false),
            ("^0.0.3", "0.0.4", false),
            ("^0.2.3", "0.2.9", true),
            ("~20.2", "20.3.0", false),
            ("18.x || >=22.0.0", "22.1.0", true),
            ("18 - 20", "20.99.0", true),
        ] {
            assert_eq!(
                matches("Node.js", range, &v(version)).unwrap(),
                yes,
                "{range}"
            );
        }
    }
    #[test]
    fn unsupported_branches_never_become_success() {
        assert!(matches("Node.js", "* || nonsense", &v("22.0.0")).is_err());
        assert!(matches("Node.js", ">=22.0.0-beta.1", &v("22.0.0")).is_err());
    }
    #[test]
    fn python_uses_pep_numeric_comparisons_not_node_partial_ranges() {
        assert!(!matches("Python", "<=3.12", &v("3.12.1")).unwrap());
        assert!(matches("Python", ">=3.10, <3.13, !=3.11.*", &v("3.12.2")).unwrap());
        assert!(!matches("Python", "~=3.11.2", &v("3.12.0")).unwrap());
        assert!(matches("Python", "~=3.11", &v("3.12.0")).unwrap());
    }
}
