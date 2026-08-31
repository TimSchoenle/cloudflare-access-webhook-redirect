use crate::Result;
use crate::error::Error;
use derive_new::new;
use regex::RegexSet;
use reqwest::Client;
use reqwest::Url;
use reqwest::header::HeaderValue;
use secrecy::{ExposeSecret, SecretString};
use std::collections::{HashMap, HashSet};

/// Everything a forwarded request needs, built once per configuration generation.
///
/// Both credentials are [`HeaderValue`]s by this point rather than secrets: they are parsed in
/// [`new`](WebHookData::new), so a token holding a byte no header may carry fails the boot instead
/// of every request.
#[derive(Getters, Debug)]
#[getset(get = "pub")]
pub struct WebHookData {
    /// Rebuilt on every reload, so a rotated credential also costs a fresh connection pool.
    client: Client,
    #[getset(skip)]
    target_host: Url,
    /// The only gate in front of the two credentials below.
    allowed_paths: AllowedPaths,
    /// Sent as `CF-Access-Client-Id` on every forwarded request.
    access_id: HeaderValue,
    /// Sent as `CF-Access-Client-Secret` on every forwarded request.
    access_secret: HeaderValue,
}

impl WebHookData {
    /// Parses both credentials into the header values they are sent as.
    ///
    /// # Errors
    /// Returns [`Error::Custom`] if either credential holds a byte that cannot appear in an HTTP
    /// header value.
    pub fn new(
        client: Client,
        target_host: Url,
        allowed_paths: AllowedPaths,
        access_id: &SecretString,
        access_secret: &SecretString,
    ) -> Result<Self> {
        let access_id = HeaderValue::from_str(access_id.expose_secret())
            .map_err(|_| Error::custom("Failed to map access id to header value"))?;
        let access_secret = HeaderValue::from_str(access_secret.expose_secret())
            .map_err(|_| Error::custom("Failed to map access secret to header value"))?;
        Ok(Self {
            client,
            target_host,
            allowed_paths,
            access_id,
            access_secret,
        })
    }

    /// Joins `path` onto `webhook.target_base`.
    ///
    /// Resolved as a relative reference, so a leading `/` replaces the base's path rather than
    /// extending it.
    ///
    /// # Errors
    /// Returns [`Error::Custom`] if the two do not join into a valid URL.
    ///
    /// # Examples
    /// ```
    /// # use cloudflare_access_webhook_redirect::data::{AllowedPaths, WebHookData};
    /// # use reqwest::{Client, Url};
    /// # use secrecy::SecretString;
    /// # use std::collections::HashMap;
    /// # let data = WebHookData::new(
    /// #     Client::new(),
    /// #     Url::parse("https://host/prefix/")?,
    /// #     AllowedPaths::new(HashMap::new())?,
    /// #     &SecretString::new(Box::from("id")),
    /// #     &SecretString::new(Box::from("secret")),
    /// # )?;
    /// assert_eq!(data.get_target_url("/webhook")?.as_str(), "https://host/webhook");
    /// assert_eq!(
    ///     data.get_target_url("webhook")?.as_str(),
    ///     "https://host/prefix/webhook"
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn get_target_url(&self, path: &str) -> Result<Url> {
        self.target_host
            .join(path)
            .map_err(|e| Error::custom(&format!("Failed to join URL: {e}")))
    }

    /// Whether `path` matches an allowed pattern that also admits `method`.
    #[must_use]
    pub fn is_allowed_path(&self, path: &str, method: &actix_web::http::Method) -> bool {
        self.allowed_paths.is_allowed(path, method)
    }
}

/// The allow list, compiled.
///
/// Each pattern is wrapped in `^` and `$`. The wrapping is textual and groups nothing, so a
/// top-level alternation escapes it: `/a|/b` compiles to `^/a|/b$`, which admits every path
/// starting `/a` and every path ending `/b`. Write the alternation as `(?:/a|/b)`.
///
/// One path may match several patterns, and it is forwarded if any one of them admits the method.
///
/// # Examples
/// ```
/// use cloudflare_access_webhook_redirect::data::{AllowedPath, AllowedPaths};
/// use std::collections::{HashMap, HashSet};
/// # use actix_web::http::Method;
///
/// let list = AllowedPaths::new(HashMap::from([(
///     "/test/".to_string(),
///     AllowedPath::new(true, HashSet::new()),
/// )]))?;
///
/// assert!(list.is_allowed("/test/", &Method::GET));
/// assert!(!list.is_allowed("/d/test/d", &Method::GET));
/// # Ok::<(), cloudflare_access_webhook_redirect::error::Error>(())
/// ```
#[derive(Getters, Debug)]
#[getset(get = "pub")]
pub struct AllowedPaths {
    /// Every anchored pattern, matched against a path in one pass.
    allowed_paths: RegexSet,
    /// What each pattern admits, keyed by the pattern as anchoring rewrote it.
    allowed_methods: HashMap<String, AllowedPath>,
}

impl AllowedPaths {
    /// Wraps each pattern in `^` and `$`, skipping an end that already has one.
    fn escape_regexes(paths: HashMap<String, AllowedPath>) -> HashMap<String, AllowedPath> {
        paths
            .into_iter()
            .map(|(mut k, v)| {
                // Escape start of the regex
                if !k.starts_with('^') {
                    k = format!("^{k}");
                }

                // Escape end of the regex
                if !k.ends_with('$') {
                    k = format!("{k}$");
                }

                (k, v)
            })
            .collect()
    }

    /// Anchors every key of `allowed_methods` and compiles them into one [`RegexSet`].
    ///
    /// # Errors
    /// Returns [`Error::Regex`] if a `webhook.paths` key is not a valid regular expression.
    pub fn new(allowed_methods: HashMap<String, AllowedPath>) -> Result<Self> {
        let allowed_methods = AllowedPaths::escape_regexes(allowed_methods);
        let allowed_paths = RegexSet::new(allowed_methods.keys())?;

        Ok(Self {
            allowed_paths,
            allowed_methods,
        })
    }

    /// Whether any anchored pattern matches `path` and admits `method`.
    #[must_use]
    pub fn is_allowed(&self, path: &str, method: &actix_web::http::Method) -> bool {
        let matches = self.allowed_paths.matches(path);
        matches
            .into_iter()
            .filter_map(|i| self.allowed_paths.patterns().get(i))
            .filter_map(|p| self.allowed_methods.get(p))
            .any(|p| p.is_allowed(method))
    }
}

/// The methods one pattern admits.
///
/// The two fields are independent. `["ALL", "GET"]` sets `all` and still lists `GET`, and `all`
/// decides the answer on its own.
#[derive(new, Getters, Debug)]
#[getset(get = "pub")]
pub struct AllowedPath {
    /// Set by the `ALL` entry, after which no method is refused.
    all: bool,
    /// The methods named one by one. `ALL` is not among them.
    methods: HashSet<actix_web::http::Method>,
}

impl AllowedPath {
    /// Whether `method` is admitted, which every method is once `all` is set.
    #[must_use]
    pub fn is_allowed(&self, method: &actix_web::http::Method) -> bool {
        self.all || self.methods.contains(method)
    }
}

#[cfg(test)]
mod tests_webhook_data {
    use crate::config::AllowedMethod;
    use crate::data::WebHookData;
    use reqwest::Client;
    use reqwest::Url;
    use secrecy::SecretString;
    use std::collections::{HashMap, HashSet};
    use std::sync::LazyLock;

    static ALL_HTTP_METHODS: LazyLock<Vec<actix_web::http::Method>> = LazyLock::new(|| {
        vec![
            actix_web::http::Method::GET,
            actix_web::http::Method::POST,
            actix_web::http::Method::PUT,
            actix_web::http::Method::DELETE,
            actix_web::http::Method::HEAD,
            actix_web::http::Method::CONNECT,
            actix_web::http::Method::OPTIONS,
            actix_web::http::Method::TRACE,
            actix_web::http::Method::PATCH,
        ]
    });
    pub struct TestWebHookData {
        client: Client,
        target_host: Url,
        allowed_paths: HashMap<String, HashSet<AllowedMethod>>,
        access_id: SecretString,
        access_secret: SecretString,
    }

    impl Default for TestWebHookData {
        fn default() -> Self {
            let mut allowed_paths = HashMap::new();
            allowed_paths.insert(
                "/test".to_string(),
                vec![AllowedMethod::GET, AllowedMethod::POST]
                    .into_iter()
                    .collect(),
            );

            Self {
                client: reqwest::Client::new(),
                target_host: Url::parse("https://example.com").unwrap(),
                allowed_paths,
                access_id: SecretString::new(Box::from("test id")),
                access_secret: SecretString::new(Box::from("test secret")),
            }
        }
    }

    impl From<TestWebHookData> for WebHookData {
        fn from(test_web_hook_data: TestWebHookData) -> Self {
            WebHookData::new(
                test_web_hook_data.client,
                test_web_hook_data.target_host,
                test_web_hook_data.allowed_paths.clone().try_into().unwrap(),
                &test_web_hook_data.access_id,
                &test_web_hook_data.access_secret,
            )
            .unwrap()
        }
    }

    #[test]
    fn test_get_target_url() {
        let base_url = Url::parse("https://example.com").unwrap();
        let web_hook_data: WebHookData = TestWebHookData {
            target_host: base_url,
            ..Default::default()
        }
        .into();

        let target_url = web_hook_data.get_target_url("/test").unwrap();
        assert_eq!(target_url.as_str(), "https://example.com/test");
    }

    #[test]
    fn test_get_target_url_trailing_slash() {
        let base_url = Url::parse("https://example.com/").unwrap();
        let web_hook_data: WebHookData = TestWebHookData {
            target_host: base_url,
            ..Default::default()
        }
        .into();

        let target_url = web_hook_data.get_target_url("/test").unwrap();
        assert_eq!(target_url.as_str(), "https://example.com/test");
    }

    #[test]
    fn test_is_allowed_path_invalid_empty() {
        let web_hook_data: WebHookData = TestWebHookData {
            allowed_paths: HashMap::new(),
            ..Default::default()
        }
        .into();

        // Invalid paths
        ALL_HTTP_METHODS.iter().for_each(|method| {
            assert!(!web_hook_data.is_allowed_path("", method));
            assert!(!web_hook_data.is_allowed_path("/", method));
            assert!(!web_hook_data.is_allowed_path("/test", method));
            assert!(!web_hook_data.is_allowed_path("/test/", method));
            assert!(!web_hook_data.is_allowed_path("/test/123", method));
        });
    }

    #[test]
    fn test_is_allowed_path_no_regex() {
        let mut paths = HashMap::new();
        paths.insert(
            "/test/".to_string(),
            vec![AllowedMethod::ALL].into_iter().collect(),
        );

        paths.insert(
            "/data/123".to_string(),
            vec![AllowedMethod::ALL].into_iter().collect(),
        );

        let web_hook_data: WebHookData = TestWebHookData {
            allowed_paths: paths,
            ..Default::default()
        }
        .into();

        // Invalid paths
        ALL_HTTP_METHODS.iter().for_each(|method| {
            assert!(!web_hook_data.is_allowed_path("", method));
            assert!(!web_hook_data.is_allowed_path("/", method));
            assert!(!web_hook_data.is_allowed_path("/api", method));
            assert!(!web_hook_data.is_allowed_path("/test/access", method));
            assert!(!web_hook_data.is_allowed_path("/data", method));
        });

        // Valid paths
        ALL_HTTP_METHODS.iter().for_each(|method| {
            assert!(web_hook_data.is_allowed_path("/test/", method));
            assert!(web_hook_data.is_allowed_path("/data/123", method));
        });
    }

    #[test]
    fn test_is_allowed_path_regex() {
        let mut paths = HashMap::new();
        paths.insert(
            "/test/".to_string(),
            vec![AllowedMethod::GET, AllowedMethod::POST]
                .into_iter()
                .collect(),
        );

        paths.insert(
            r"/data/\d*/private".to_string(),
            vec![AllowedMethod::ALL].into_iter().collect(),
        );

        let web_hook_data: WebHookData = TestWebHookData {
            allowed_paths: paths,
            ..Default::default()
        }
        .into();

        // Invalid paths
        ALL_HTTP_METHODS.iter().for_each(|method| {
            assert!(!web_hook_data.is_allowed_path("", method));
            assert!(!web_hook_data.is_allowed_path("/", method));
            assert!(!web_hook_data.is_allowed_path("/api", method));
            assert!(!web_hook_data.is_allowed_path("/data", method));
            assert!(!web_hook_data.is_allowed_path("/data/123/", method));
            assert!(!web_hook_data.is_allowed_path("/data/123/test", method));
            assert!(!web_hook_data.is_allowed_path("/data/abc/private", method));
        });

        // Check /test/ path
        ALL_HTTP_METHODS.iter().for_each(|method| {
            if method == actix_web::http::Method::GET || method == actix_web::http::Method::POST {
                assert!(web_hook_data.is_allowed_path("/test/", method));
            } else {
                assert!(!web_hook_data.is_allowed_path("/test/", method));
            }
        });

        // Check /data/123/private path
        ALL_HTTP_METHODS.iter().for_each(|method| {
            assert!(web_hook_data.is_allowed_path("/data/123/private", method));
        });
    }

    #[test]
    fn test_is_allowed_path_correctness() {
        let mut paths = HashMap::new();
        paths.insert(
            "/test/".to_string(),
            vec![AllowedMethod::GET].into_iter().collect(),
        );

        let web_hook_data: WebHookData = TestWebHookData {
            allowed_paths: paths,
            ..Default::default()
        }
        .into();

        // Invalid paths
        assert!(!web_hook_data.is_allowed_path("/d/test/", &actix_web::http::Method::GET));
        assert!(!web_hook_data.is_allowed_path("/d/test/d", &actix_web::http::Method::GET));
    }
}

#[cfg(test)]
mod tests_allowed_paths {
    use crate::data::{AllowedPath, AllowedPaths};
    use std::collections::HashMap;

    fn create_map(paths: Vec<&str>) -> HashMap<String, AllowedPath> {
        let mut map = HashMap::new();
        for path in paths {
            map.insert(
                path.to_string(),
                AllowedPath::new(
                    false,
                    vec![actix_web::http::Method::GET].into_iter().collect(),
                ),
            );
        }

        map
    }

    fn verify_map(map: &HashMap<String, AllowedPath>) {
        for key in map.keys() {
            // Assert correct ends and starts
            assert!(key.starts_with('^'));
            assert!(key.ends_with('$'));

            // Make sure that we don't add an extra ^ or $ to the regex
            assert!(!key.starts_with("^^"));
            assert!(!key.ends_with("$$"));
        }
    }

    fn verify_paths(paths: Vec<&str>) {
        let input = create_map(paths);
        let converted = AllowedPaths::escape_regexes(input);
        verify_map(&converted);
    }

    #[test]
    fn test_escape_regexes_empty() {
        verify_paths(vec![r"/test/", "", r"/data/\d*/private"]);
    }

    #[test]
    fn test_escape_regexes_escaped_start() {
        verify_paths(vec![r"^/test/", "^", r"^/data/\d*/private"]);
    }

    #[test]
    fn test_escape_regexes_escaped_end() {
        verify_paths(vec![r"/test/$", "$", r"/data/\d*/private$"]);
    }

    #[test]
    fn test_escape_regexes_escaped() {
        verify_paths(vec![r"^/test/$", "^$", r"^/data/\d*/private$"]);
    }
}
