use crate::data::{AllowedPath, AllowedPaths};
use reqwest::Url;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use crate::error::Error;

const DEFAULT_SERVER_HOST: &str = "127.0.0.1";
const DEFAULT_SERVER_PORT: u16 = 8080;

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub cloudflare: CloudFlareConfig,
    pub webhook: WebhookConfig,
}

#[derive(Debug, serde::Deserialize)]
pub struct CloudFlareConfig {
    pub client_id: SecretString,
    pub client_secret: SecretString,
}

#[derive(Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, serde::Deserialize)]
pub struct WebhookConfig {
    #[serde(deserialize_with = "deserialize_url_from_string")]
    pub target_base: Url,
    // Regex path: Allowed methods
    #[serde(deserialize_with = "deserialize_paths_from_string")]
    pub paths: HashMap<String, HashSet<AllowedMethod>>,
}

impl Config {
    pub fn get_configuration() -> crate::Result<Self> {
        config::Config::builder()
            .add_source(config::Environment::default().try_parsing(true))
            .set_default("server.host", DEFAULT_SERVER_HOST)?
            .set_default("server.port", DEFAULT_SERVER_PORT)?
            .build()
            .map_err(|e| Error::custom(format!("Can't parse config: {e}")))?
            .try_deserialize::<Config>()
            .map_err(|e| Error::custom(format!("Failed to deserialize configuration: {e}")))
    }
}

pub fn deserialize_url_from_string<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let string: String = Deserialize::deserialize(deserializer)?;
    Url::parse(&string).map_err(serde::de::Error::custom)
}

pub fn deserialize_paths_from_string<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, HashSet<AllowedMethod>>, D::Error>
where
    D: Deserializer<'de>,
{
    let string: String = Deserialize::deserialize(deserializer)?;

    let values: Result<HashMap<String, HashSet<AllowedMethod>>, D::Error> = string
        .split("; ")
        .map(|s| {
            let mut split = s.split(':');

            let path = match split.next() {
                Some(s) => Ok(s),
                None => Err(serde::de::Error::custom("Path is missing")),
            }?;

            let methods = match split.next() {
                Some(s) => Ok(s),
                None => Err(serde::de::Error::custom("Methods are missing")),
            }?;

            let methods: Result<HashSet<AllowedMethod>, _> = methods
                .split(',')
                .map(|s| s.to_uppercase())
                .map(|s| AllowedMethod::try_from(&s))
                .collect();

            Ok((path.to_string(), methods.map_err(serde::de::Error::custom)?))
        })
        .collect();

    values
}

#[derive(Debug, serde::Deserialize, Eq, PartialEq, Hash, Clone)]
pub enum AllowedMethod {
    ALL,
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
}

impl AllowedMethod {
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

impl TryFrom<&String> for AllowedMethod {
    type Error = crate::Error;

    fn try_from(value: &String) -> Result<Self, Self::Error> {
        match value.to_uppercase().as_str() {
            "ALL" => Ok(AllowedMethod::ALL),
            "GET" => Ok(AllowedMethod::GET),
            "POST" => Ok(AllowedMethod::POST),
            "PUT" => Ok(AllowedMethod::PUT),
            "PATCH" => Ok(AllowedMethod::PATCH),
            "DELETE" => Ok(AllowedMethod::DELETE),
            _ => Err(Error::custom(format!("Unknown method: {}", value))),
        }
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

impl TryFrom<AllowedMethod> for actix_web::http::Method {
    type Error = Error;

    fn try_from(value: AllowedMethod) -> Result<Self, Self::Error> {
        if value == AllowedMethod::ALL {
            return Err(Error::custom(
                "Can't convert ALL to actix_web::http::Method",
            ));
        }

        actix_web::http::Method::from_str(value.name()).map_err(|e| {
            Error::custom(format!(
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
                assert!(result.is_err(), "Expected error, got: {:?}", result);
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

        let allowed_paths: crate::config::AllowedPaths = paths.try_into().unwrap();
        assert!(allowed_paths.is_allowed("/test", &actix_web::http::Method::GET));
        assert!(allowed_paths.is_allowed("/test", &actix_web::http::Method::PUT));
    }

    #[test]
    fn test_map_allowed_method_try_get() {
        let mut paths = HashMap::new();

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::GET);
        paths.insert("/test".to_string(), methods);

        let allowed_paths: crate::config::AllowedPaths = paths.try_into().unwrap();
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

        let allowed_path: crate::config::AllowedPath = set.try_into().unwrap();
        assert!(allowed_path.all);
        assert_eq!(allowed_path.methods.len(), 5);
    }

    #[test]
    fn test_set_allowed_method_try_into_minimal_no_all() {
        let mut set = HashSet::new();
        set.insert(AllowedMethod::GET);

        let allowed_path: crate::config::AllowedPath = set.try_into().unwrap();
        assert!(!allowed_path.all);
        assert_eq!(allowed_path.methods.len(), 1);
        assert!(allowed_path.methods.contains(&actix_web::http::Method::GET));
    }

    #[test]
    fn test_set_allowed_method_try_into_minimal_with_all() {
        let mut set = HashSet::new();
        set.insert(AllowedMethod::ALL);

        let allowed_path: crate::config::AllowedPath = set.try_into().unwrap();
        assert!(allowed_path.all);
        assert_eq!(allowed_path.methods.len(), 0);
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
mod tests {
    use crate::config::{AllowedMethod, Config};
    use secrecy::ExposeSecret;
    use std::collections::{HashMap, HashSet};

    const ENV_SERVER_HOST: &str = "SERVER.HOST";
    const ENV_SERVER_PORT: &str = "SERVER.PORT";

    const ENV_CLOUDFLARE_CLIENT_ID: &str = "CLOUDFLARE.CLIENT_ID";
    const ENV_CLOUDFLARE_CLIENT_SECRET: &str = "CLOUDFLARE.CLIENT_SECRET";

    const ENV_WEBHOOK_TARGET_BASE: &str = "WEBHOOK.TARGET_BASE";
    const ENV_WEBHOOK_PATHS: &str = "WEBHOOK.PATHS";

    const CORRECT_SERVER_HOST: &str = "0.0.0.0";
    const CORRECT_SERVER_PORT: &str = "8080";

    const CORRECT_CLOUDFLARE_CLIENT_ID: &str = "client_id";
    const CORRECT_CLOUDFLARE_CLIENT_SECRET: &str = "client_secret";

    const CORRECT_WEBHOOK_TARGET_BASE: &str = "https://example.com/";
    const CORRECT_WEBHOOK_PATHS: &str = "/test:ALL";

    #[test]
    fn test_get_configurations_minimal_correct() -> Result<(), Box<dyn std::error::Error>> {
        let config = temp_env::with_vars(
            vec![
                (ENV_CLOUDFLARE_CLIENT_ID, Some(CORRECT_CLOUDFLARE_CLIENT_ID)),
                (
                    ENV_CLOUDFLARE_CLIENT_SECRET,
                    Some(CORRECT_CLOUDFLARE_CLIENT_SECRET),
                ),
                (ENV_WEBHOOK_TARGET_BASE, Some(CORRECT_WEBHOOK_TARGET_BASE)),
                (ENV_WEBHOOK_PATHS, Some(CORRECT_WEBHOOK_PATHS)),
            ],
            Config::get_configuration,
        )?;

        assert_eq!(
            config.cloudflare.client_id.expose_secret(),
            CORRECT_CLOUDFLARE_CLIENT_ID
        );
        assert_eq!(
            config.cloudflare.client_secret.expose_secret(),
            CORRECT_CLOUDFLARE_CLIENT_SECRET
        );

        assert_eq!(
            config.webhook.target_base.as_str(),
            CORRECT_WEBHOOK_TARGET_BASE
        );

        let mut paths = HashMap::new();

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::ALL);
        paths.insert("/test".to_string(), methods);

        assert_eq!(config.webhook.paths, paths);

        Ok(())
    }

    #[test]
    fn test_get_configurations_full_correct() -> Result<(), Box<dyn std::error::Error>> {
        let config = temp_env::with_vars(
            vec![
                (ENV_SERVER_HOST, Some(CORRECT_SERVER_HOST)),
                (ENV_SERVER_PORT, Some(CORRECT_SERVER_PORT)),
                (ENV_CLOUDFLARE_CLIENT_ID, Some(CORRECT_CLOUDFLARE_CLIENT_ID)),
                (
                    ENV_CLOUDFLARE_CLIENT_SECRET,
                    Some(CORRECT_CLOUDFLARE_CLIENT_SECRET),
                ),
                (ENV_WEBHOOK_TARGET_BASE, Some(CORRECT_WEBHOOK_TARGET_BASE)),
                (
                    ENV_WEBHOOK_PATHS,
                    Some(r"/test:All; /test2:GET; /test\d*:POST,PUT"),
                ),
            ],
            Config::get_configuration,
        )?;

        assert_eq!(config.server.host, CORRECT_SERVER_HOST);
        assert_eq!(config.server.port, 8080u16);

        assert_eq!(
            config.cloudflare.client_id.expose_secret(),
            CORRECT_CLOUDFLARE_CLIENT_ID
        );
        assert_eq!(
            config.cloudflare.client_secret.expose_secret(),
            CORRECT_CLOUDFLARE_CLIENT_SECRET
        );

        assert_eq!(
            config.webhook.target_base.as_str(),
            CORRECT_WEBHOOK_TARGET_BASE
        );

        let mut paths = HashMap::new();

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::ALL);
        paths.insert("/test".to_string(), methods);

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::GET);
        paths.insert("/test2".to_string(), methods);

        let mut methods = HashSet::new();
        methods.insert(AllowedMethod::POST);
        methods.insert(AllowedMethod::PUT);
        paths.insert(r"/test\d*".to_string(), methods);

        assert_eq!(config.webhook.paths, paths);

        Ok(())
    }
}
