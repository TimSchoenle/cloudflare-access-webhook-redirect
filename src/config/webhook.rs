//! What is proxied, and where to.

use crate::data::{AllowedPath, AllowedPaths};
use crate::error::Error;
use reqwest::Url;
use serde::{Deserialize, Deserializer};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

/// The upstream and the paths allowed to reach it.
#[derive(Debug, Clone, Deserialize, Getters)]
#[cfg_attr(feature = "config-schema", derive(terrace_config::schema::Describe))]
#[getset(get = "pub")]
#[serde(deny_unknown_fields)]
pub struct WebhookConfig {
    /// Base URL of the Cloudflare Access protected service every allowed path is joined onto.
    #[serde(deserialize_with = "deserialize_url_from_string")]
    target_base: Url,
    /// Path regex to the methods allowed on it.
    ///
    /// A table rather than the packed string the environment layer forced, so a pattern
    /// containing the old separators is no longer unspellable:
    ///
    /// ```toml
    /// [webhook.paths]
    /// "/webhook/.*" = ["ALL"]
    /// "/api/public/.*" = ["GET", "POST"]
    /// ```
    // Spelled out rather than read off [`AllowedMethod`]'s variants, which is what the bare
    // `element_values` this replaced did. A derive reports serde's *variant* names — the
    // `UPPERCASE` half — and does not read `#[serde(alias = "…")]`, so the published list left
    // out the six lowercase spellings the deserialiser accepts and the schema refused
    // `["get"]`, which loads. `element_values("…")` from terrace-config v0.11.0 is what can say
    // both halves. The list is the twelve spellings `AllowedMethod`'s `Deserialize` derive
    // matches: one per variant from `#[serde(rename_all = "UPPERCASE")]`, one per variant from
    // its `#[serde(alias)]`, and nothing else — `FromStr` below is not in this path.
    #[cfg_attr(
        feature = "config-schema",
        config(element_values(
            "ALL", "all", "GET", "get", "POST", "post", "PUT", "put", "PATCH", "patch", "DELETE",
            "delete"
        ))
    )]
    paths: HashMap<String, HashSet<AllowedMethod>>,
}

fn deserialize_url_from_string<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let string: String = Deserialize::deserialize(deserializer)?;
    Url::parse(&string).map_err(serde::de::Error::custom)
}

/// A method a path may be proxied with, or [`AllowedMethod::ALL`] for every one of them.
///
/// Deserialised by variant name, uppercase or lowercase — `"get"` and `"GET"` are the same value,
/// through the `UPPERCASE` spelling and a lowercase [`serde(alias)`](serde::Deserialize) on each
/// variant, rather than through [`FromStr`]. [`FromStr`] stays only for the call sites below that
/// parse a method outside of `serde` (a set converted from configuration, and the reverse
/// direction back to [`actix_web::http::Method`]). Operators write these by hand.
///
/// Deliberately **not** `Describe`. That derive builds `Values` from the variant names alone —
/// `#[serde(rename_all)]` is applied, `#[serde(alias)]` is not — so the twelve spellings this
/// type accepts would reach a schema as six, and a file saying `["get"]` would be refused by a
/// document describing a loader that takes it. The spellings are asserted where they can be
/// stated in full instead, on [`WebhookConfig::paths`] with `#[config(element_values("…"))]`.
/// Without a `Values` implementation here, a bare `#[config(element_values)]` cannot compile,
/// so the six-spelling version cannot come back by accident.
///
/// Every forwarded request keeps its query string. Whether it keeps its body depends on which of
/// these it is.
#[derive(Debug, serde::Deserialize, Eq, PartialEq, Hash, Clone)]
#[cfg_attr(feature = "config-schema", derive(serde::Serialize))]
#[serde(rename_all = "UPPERCASE")]
pub enum AllowedMethod {
    /// Every method, so any other entry in the same list is ignored.
    #[serde(alias = "all")]
    ALL,
    /// Forwarded without its body.
    #[serde(alias = "get")]
    GET,
    /// Forwarded with its body.
    #[serde(alias = "post")]
    POST,
    /// Forwarded with its body.
    #[serde(alias = "put")]
    PUT,
    /// Forwarded with its body.
    #[serde(alias = "patch")]
    PATCH,
    /// Forwarded without its body.
    #[serde(alias = "delete")]
    DELETE,
}

impl AllowedMethod {
    /// The uppercase spelling, `"ALL"` included.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            AllowedMethod::ALL => "ALL",
            AllowedMethod::GET => "GET",
            AllowedMethod::POST => "POST",
            AllowedMethod::PUT => "PUT",
            AllowedMethod::PATCH => "PATCH",
            AllowedMethod::DELETE => "DELETE",
        }
    }
}

/// Kept for the call sites that parse a method outside of `serde` — [`TryFrom<String>`] below,
/// and the reverse conversion to [`actix_web::http::Method`] — rather than folded into the
/// `Deserialize` derive above, which now only accepts the two spellings [`serde(alias)`] names
/// per variant.
///
/// [`serde(alias)`]: serde::Deserialize
impl FromStr for AllowedMethod {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_uppercase().as_str() {
            "ALL" => Ok(AllowedMethod::ALL),
            "GET" => Ok(AllowedMethod::GET),
            "POST" => Ok(AllowedMethod::POST),
            "PUT" => Ok(AllowedMethod::PUT),
            "PATCH" => Ok(AllowedMethod::PATCH),
            "DELETE" => Ok(AllowedMethod::DELETE),
            _ => Err(Error::custom(&format!("Unknown method: {value}"))),
        }
    }
}

impl TryFrom<String> for AllowedMethod {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<&String> for AllowedMethod {
    type Error = Error;

    fn try_from(value: &String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<HashMap<String, HashSet<AllowedMethod>>> for AllowedPaths {
    type Error = Error;

    fn try_from(value: HashMap<String, HashSet<AllowedMethod>>) -> Result<Self, Self::Error> {
        let mut allowed_paths = HashMap::with_capacity(value.len());
        for (path, methods) in value {
            allowed_paths.insert(path, methods.try_into()?);
        }

        AllowedPaths::new(allowed_paths)
    }
}

impl TryFrom<HashSet<AllowedMethod>> for AllowedPath {
    type Error = Error;

    fn try_from(value: HashSet<AllowedMethod>) -> Result<Self, Self::Error> {
        let mut filtered_methods = HashSet::with_capacity(value.len());
        let mut all = false;
        for method in value {
            if method == AllowedMethod::ALL {
                all = true;
                continue;
            }

            filtered_methods.insert(method.try_into()?);
        }

        Ok(AllowedPath::new(all, filtered_methods))
    }
}

/// [`AllowedMethod::ALL`] is the one input this refuses: it stands for a set, and actix has no
/// method that means every method.
impl TryFrom<AllowedMethod> for actix_web::http::Method {
    type Error = Error;

    fn try_from(value: AllowedMethod) -> Result<Self, Self::Error> {
        if value == AllowedMethod::ALL {
            return Err(Error::custom(
                "Can't convert ALL to actix_web::http::Method",
            ));
        }

        actix_web::http::Method::from_str(value.name()).map_err(|e| {
            Error::custom(&format!(
                "Can't convert method to actix_web::http::Method: {} | {}",
                e,
                value.name()
            ))
        })
    }
}

#[cfg(test)]
mod tests_try_from {
    use crate::config::AllowedMethod;
    use std::collections::{HashMap, HashSet};

    fn compare_option_with_result<T>(expected: Option<T>, result: crate::Result<T>)
    where
        T: std::fmt::Debug + std::cmp::PartialEq,
    {
        match expected {
            Some(expected) => {
                assert!(result.is_ok());
                assert_eq!(result.unwrap(), expected);
            }
            None => {
                assert!(result.is_err(), "Expected error, got: {result:?}");
            }
        }
    }

    fn test_string_to_allowed_method(input: &String, expected: Option<AllowedMethod>) {
        let method: crate::Result<AllowedMethod> = input.try_into();
        compare_option_with_result(expected, method);
    }

    fn test_allowed_method_to_http_method(
        allowed_method: AllowedMethod,
        http_method: Option<actix_web::http::Method>,
    ) {
        let method: crate::Result<actix_web::http::Method> = allowed_method.try_into();
        compare_option_with_result(http_method, method);
    }

    #[test]
    fn test_string_to_allowed_method_upper_case() {
        test_string_to_allowed_method(&"ALL".to_string(), Some(AllowedMethod::ALL));
        test_string_to_allowed_method(&"GET".to_string(), Some(AllowedMethod::GET));
        test_string_to_allowed_method(&"POST".to_string(), Some(AllowedMethod::POST));
        test_string_to_allowed_method(&"PUT".to_string(), Some(AllowedMethod::PUT));
        test_string_to_allowed_method(&"PATCH".to_string(), Some(AllowedMethod::PATCH));
        test_string_to_allowed_method(&"DELETE".to_string(), Some(AllowedMethod::DELETE));
    }

    #[test]
    fn test_string_to_allowed_method_lower_case() {
        test_string_to_allowed_method(&"all".to_string(), Some(AllowedMethod::ALL));
        test_string_to_allowed_method(&"get".to_string(), Some(AllowedMethod::GET));
        test_string_to_allowed_method(&"post".to_string(), Some(AllowedMethod::POST));
        test_string_to_allowed_method(&"put".to_string(), Some(AllowedMethod::PUT));
        test_string_to_allowed_method(&"patch".to_string(), Some(AllowedMethod::PATCH));
        test_string_to_allowed_method(&"delete".to_string(), Some(AllowedMethod::DELETE));
    }

    #[test]
    fn test_string_to_allowed_method_invalid() {
        test_string_to_allowed_method(&"test".to_string(), None);
        test_string_to_allowed_method(&"GETT".to_string(), None);
        test_string_to_allowed_method(&"gett".to_string(), None);
    }

    #[test]
    fn test_map_allowed_method_try_all() {
        let mut paths = HashMap::new();

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::ALL);
        paths.insert("/test".to_string(), methods);

        let allowed_paths: crate::data::AllowedPaths = paths.try_into().unwrap();
        assert!(allowed_paths.is_allowed("/test", &actix_web::http::Method::GET));
        assert!(allowed_paths.is_allowed("/test", &actix_web::http::Method::PUT));
    }

    #[test]
    fn test_map_allowed_method_try_get() {
        let mut paths = HashMap::new();

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::GET);
        paths.insert("/test".to_string(), methods);

        let allowed_paths: crate::data::AllowedPaths = paths.try_into().unwrap();
        assert!(allowed_paths.is_allowed("/test", &actix_web::http::Method::GET));
        assert!(!allowed_paths.is_allowed("/test", &actix_web::http::Method::PUT));
    }

    #[test]
    fn test_set_allowed_method_try_into_full() {
        let mut set = HashSet::new();
        set.insert(AllowedMethod::ALL);
        set.insert(AllowedMethod::GET);
        set.insert(AllowedMethod::POST);
        set.insert(AllowedMethod::PUT);
        set.insert(AllowedMethod::PATCH);
        set.insert(AllowedMethod::DELETE);

        let allowed_path: crate::data::AllowedPath = set.try_into().unwrap();
        assert!(allowed_path.all());
        assert_eq!(allowed_path.methods().len(), 5);
    }

    #[test]
    fn test_set_allowed_method_try_into_minimal_no_all() {
        let mut set = HashSet::new();
        set.insert(AllowedMethod::GET);

        let allowed_path: crate::data::AllowedPath = set.try_into().unwrap();
        assert!(!allowed_path.all());
        assert_eq!(allowed_path.methods().len(), 1);
        assert!(
            allowed_path
                .methods()
                .contains(&actix_web::http::Method::GET)
        );
    }

    #[test]
    fn test_set_allowed_method_try_into_minimal_with_all() {
        let mut set = HashSet::new();
        set.insert(AllowedMethod::ALL);

        let allowed_path: crate::data::AllowedPath = set.try_into().unwrap();
        assert!(allowed_path.all());
        assert_eq!(allowed_path.methods().len(), 0);
    }

    #[test]
    fn test_allowed_method_try_into() {
        test_allowed_method_to_http_method(AllowedMethod::ALL, None);
        test_allowed_method_to_http_method(AllowedMethod::GET, Some(actix_web::http::Method::GET));
        test_allowed_method_to_http_method(
            AllowedMethod::POST,
            Some(actix_web::http::Method::POST),
        );
        test_allowed_method_to_http_method(AllowedMethod::PUT, Some(actix_web::http::Method::PUT));
        test_allowed_method_to_http_method(
            AllowedMethod::PATCH,
            Some(actix_web::http::Method::PATCH),
        );
        test_allowed_method_to_http_method(
            AllowedMethod::DELETE,
            Some(actix_web::http::Method::DELETE),
        );
    }
}

#[cfg(test)]
mod tests_deserialize {
    use super::{AllowedMethod, WebhookConfig};
    use figment::providers::Format;
    use std::collections::HashSet;

    /// The error is stringified because `figment::Error` is large enough to trip
    /// `clippy::result_large_err`, and every assertion below reads its message anyway.
    fn deserialize(toml: &str) -> Result<WebhookConfig, String> {
        figment::Figment::from(figment::providers::Toml::string(toml))
            .extract()
            .map_err(|e| e.to_string())
    }

    #[test]
    fn a_path_table_becomes_the_method_sets() {
        let config = deserialize(
            r#"
            target_base = "https://example.com/"

            [paths]
            "/webhook/.*" = ["ALL"]
            "/api/public/.*" = ["get", "POST"]
            "#,
        )
        .unwrap();

        assert_eq!(config.target_base().as_str(), "https://example.com/");
        assert_eq!(config.paths().len(), 2);
        assert_eq!(
            config.paths().get("/webhook/.*"),
            Some(&HashSet::from([AllowedMethod::ALL]))
        );
        assert_eq!(
            config.paths().get("/api/public/.*"),
            Some(&HashSet::from([AllowedMethod::GET, AllowedMethod::POST]))
        );
    }

    /// The packed `"<regex>:<methods>; …"` string the environment layer forced could not
    /// express a pattern containing `:` or `; `. A TOML key can.
    #[test]
    fn a_path_regex_may_contain_the_old_separators() {
        let config = deserialize(
            r#"
            target_base = "https://example.com/"

            [paths]
            "/webhook/[a-z]{2}; ?:[0-9]*" = ["POST"]
            "#,
        )
        .unwrap();

        assert!(config.paths().contains_key("/webhook/[a-z]{2}; ?:[0-9]*"));
    }

    #[test]
    fn an_unknown_method_is_refused() {
        let error = deserialize(
            r#"
            target_base = "https://example.com/"

            [paths]
            "/webhook/.*" = ["FETCH"]
            "#,
        )
        .expect_err("FETCH is not a method");

        assert!(
            error.contains("FETCH"),
            "the error must name the value: {error}"
        );
    }

    #[test]
    fn a_malformed_target_base_is_refused() {
        let error = deserialize(
            r#"
            target_base = "not-a-url"

            [paths]
            "/webhook/.*" = ["ALL"]
            "#,
        )
        .expect_err("not a URL");

        assert!(
            error.contains("target_base"),
            "the error must name the key: {error}"
        );
    }
}
