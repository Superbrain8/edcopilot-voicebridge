//! The origin of a service URL, read with the URL parser of the HTTP client.
//!
//! The HTTP client (`reqwest`) reads a URL with the `url` crate, by the
//! WHATWG rules. This module uses the same parser. Thus the host that a check
//! sees is the host that the request goes to. Do not read a URL by hand.

use url::{Host, Url};

/// The URL that the cloud engine requests for its base URL.
pub fn request_url(base_url: &str) -> String {
    format!(
        "{}/audio/transcriptions",
        base_url.trim().trim_end_matches('/')
    )
}

/// The parsed URL when its scheme is `http` or `https` and it has a host.
fn parse_http(url: &str) -> Option<Url> {
    let url = Url::parse(url).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty()))
        .then_some(url)
}

/// The origin of an `http` or `https` URL: the scheme, the host and the
/// port, for example `https://api.openai.com`. The parser makes the host
/// lower case and writes a name with non-ASCII letters in punycode. The
/// default port of the scheme is not shown, so `:443` and no port give the
/// same origin for `https`. `None` for a URL that does not parse, for
/// another scheme, and for a URL without a host.
pub fn url_origin(url: &str) -> Option<String> {
    let url = parse_http(url)?;
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url.host_str()?.to_ascii_lowercase();
    let default = if scheme == "https" { 443 } else { 80 };
    Some(match url.port_or_known_default() {
        Some(port) if port != default => format!("{scheme}://{host}:{port}"),
        _ => format!("{scheme}://{host}"),
    })
}

/// True when the host of the URL is this computer: `localhost`, an IPv4
/// loopback address, or the IPv6 loopback address.
pub fn is_loopback_url(url: &str) -> bool {
    match parse_http(url).as_ref().and_then(Url::host) {
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Why a base URL is not safe to use, or `None`. The text never repeats
/// user data of the URL: it shows the scheme and the host only, and only
/// when the URL parsed.
pub fn base_url_problem(base_url: &str) -> Option<String> {
    let parsed = Url::parse(base_url).ok();
    let shown = || {
        parsed
            .as_ref()
            .and_then(|u| u.host_str().map(|h| format!("{}://{h}", u.scheme())))
    };
    let reason = if base_url
        .chars()
        .any(|c| c == '\\' || c.is_whitespace() || c.is_control())
    {
        "it must not hold a backslash, a space or a control character"
    } else if parsed.is_none() {
        return Some("is not a valid http or https URL".to_string());
    } else if !matches!(parsed.as_ref()?.scheme(), "http" | "https") {
        "it must use http or https"
    } else if parsed.as_ref()?.host_str().is_none_or(str::is_empty) {
        return Some("has no host, it is not a valid http or https URL".to_string());
    } else if !parsed.as_ref()?.username().is_empty() || parsed.as_ref()?.password().is_some() {
        "it must not hold a user name or a password"
    } else {
        return None;
    };
    Some(match shown() {
        Some(origin) => format!("({origin}) is not valid: {reason}"),
        None => format!("is not valid: {reason}"),
    })
}

/// The problem of a base URL that holds a user name or a password, else
/// `None`. Such a value can be a secret, so the configuration must not hold
/// it, also when the cloud engine is off. The text shows the scheme and the
/// host only.
pub fn base_url_user_info_problem(base_url: &str) -> Option<String> {
    let url = Url::parse(base_url).ok()?;
    if url.username().is_empty() && url.password().is_none() {
        return None;
    }
    let reason = "it must not hold a user name or a password";
    Some(match url.host_str() {
        Some(host) => format!("({}://{host}) is not valid: {reason}", url.scheme()),
        None => format!("is not valid: {reason}"),
    })
}

/// The base URL as a page can show it, when the URL holds a user name, a
/// password, a query or a fragment: the scheme, the host, the port and the
/// path only. A URL that does not parse and has `@`, `?` or `#` gives the
/// empty text. `None` when the URL holds none of these parts: then the
/// page can show it as it is.
pub fn base_url_without_secrets(base_url: &str) -> Option<String> {
    match Url::parse(base_url) {
        Ok(url) => {
            let plain = url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none();
            if plain {
                return None;
            }
            let host = url.host_str().unwrap_or_default();
            let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
            Some(format!("{}://{host}{port}{}", url.scheme(), url.path()))
        }
        Err(_) => base_url.contains(['@', '?', '#']).then(String::new),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_url_without_secrets_keeps_only_scheme_host_port_and_path() {
        let cases = [
            (
                "https://user:pw@host.example/v1?api_key=abc#frag",
                Some("https://host.example/v1"),
            ),
            (
                "https://user@host.example:8443/v1",
                Some("https://host.example:8443/v1"),
            ),
            ("http://host.example/v1?x=1", Some("http://host.example/v1")),
            (
                "https://host.example/v1#frag",
                Some("https://host.example/v1"),
            ),
            ("https://user:pw@", Some("")),
            ("https://host.example/v1", None),
            ("http://localhost:11434/v1", None),
            ("not a url", None),
            ("", None),
        ];
        for (url, shown) in cases {
            assert_eq!(base_url_without_secrets(url).as_deref(), shown, "{url}");
        }
    }

    #[test]
    fn test_origin_rules() {
        assert_eq!(
            url_origin("https://api.openai.com:443/v1").as_deref(),
            Some("https://api.openai.com")
        );
        assert_eq!(
            url_origin("https://API.OPENAI.COM"),
            url_origin("https://api.openai.com")
        );
        assert_eq!(
            url_origin("http://example.org:8080").as_deref(),
            Some("http://example.org:8080")
        );
        assert_eq!(url_origin("ftp://example.org"), None);
        assert_eq!(url_origin("mailto:a@example.org"), None);
        assert!(is_loopback_url("http://127.0.0.2:9/"));
        assert!(is_loopback_url("http://LOCALHOST/"));
        assert!(!is_loopback_url("http://localhost.evil.example/"));
    }

    #[test]
    fn test_user_info_problem_is_only_for_user_data() {
        for url in [
            "",
            "not a url",
            "ftp://x.example",
            "https://x.example\\y",
            "https://x.example/v1",
        ] {
            assert_eq!(base_url_user_info_problem(url), None, "{url}");
        }
        let problem =
            base_url_user_info_problem("https://SECRETUSER:SECRETPW@x.example/v1").unwrap();
        assert!(!problem.contains("SECRET"), "{problem}");
        assert!(problem.contains("user name or a password"), "{problem}");
        assert!(base_url_user_info_problem("https://SECRET@x.example").is_some());
    }

    #[test]
    fn test_base_url_problem_never_shows_user_data() {
        for url in [
            "https://SECRETUSER:SECRETPW@evil.example/v1",
            "https://SECRETUSER@evil.example/v1",
            "https://SECRET\\@evil.example/v1",
            "https://SECRET evil.example/v1",
        ] {
            let problem = base_url_problem(url).expect(url);
            assert!(!problem.contains("SECRET"), "{url}: {problem}");
        }
        assert_eq!(base_url_problem("https://api.openai.com/v1"), None);
        assert_eq!(
            base_url_problem("https://u:p@evil.example/v1").as_deref(),
            Some("(https://evil.example) is not valid: it must not hold a user name or a password")
        );
    }
}
