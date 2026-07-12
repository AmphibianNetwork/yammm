//! External API clients for Modrinth, CurseForge, Minecraft, GitHub, etc.
//!
//! These clients handle rate limiting, authentication, and raw JSON/REST parsing.
//! The `ModSourceProvider` trait implementations live in `providers/`.
//!
//! The `define_api_client!` macro generates boilerplate: struct, `new()`,
//! `with_client()`, `with_base_url()`, `Default` impl, and shared HTTP helpers
//! via the `ApiClient` trait.

pub mod adoptium;
pub mod curseforge;
pub mod error;
pub mod forge;
pub mod github;
pub mod http_cache;
pub mod installer;
pub mod loader;
pub mod minecraft;
pub mod modrinth;
pub mod neoforge;
pub mod retry;
pub mod streaming;

pub use curseforge::CurseForgeClient;
pub use error::ApiError;
pub use forge::ForgeClient;
pub use github::GitHubClient;
pub use loader::{FabricClient, QuiltClient};
pub use minecraft::MinecraftClient;
pub use modrinth::{ModrinthClient, ModrinthSearchHit};
pub use neoforge::NeoForgeClient;

/// Default per-request connect timeout for HTTP clients.
pub const DEFAULT_CONNECT_TIMEOUT: std::time::Duration =
	std::time::Duration::from_secs(10);

/// Default total-request timeout for HTTP clients. Applies from connect
/// through to the last byte of the response body; long-running streaming
/// downloads should run on a client with this timeout disabled or extended.
pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration =
	std::time::Duration::from_secs(30);

/// Build a `reqwest::Client` with the project's standard timeouts and
/// user-agent.
///
/// This is the path used by [`define_api_client!`]-generated `new()`
/// constructors and by the [`crate::app::AppContext`] failure fallback,
/// so a forgotten `with_client()` does not silently produce a no-timeout
/// client.
///
/// # Panics
///
/// Panics if `reqwest::ClientBuilder::build` fails — this only happens when
/// the system TLS backend is unavailable, which is unrecoverable for any
/// network-using path in the binary.
pub fn default_http_client() -> reqwest::Client {
	reqwest::Client::builder()
		.user_agent(format!(
			"AmphibianNetwork/yammm/{} (contact@amphibian.network)",
			env!("CARGO_PKG_VERSION")
		))
		.connect_timeout(DEFAULT_CONNECT_TIMEOUT)
		.timeout(DEFAULT_REQUEST_TIMEOUT)
		.build()
		.expect("HTTP client initialization failed — TLS backend unavailable")
}

/// Shared HTTP helpers for all API clients.
#[allow(async_fn_in_trait)]
pub trait ApiClient: private::Sealed + Send + Sync {
	fn http_client(&self) -> &reqwest::Client;

	async fn send_retried(
		&self,
		url: &str,
		extra_headers: Vec<(&'static str, String)>,
	) -> Result<reqwest::Response, ApiError> {
		crate::api::retry::send_retried_mapped(
			self.http_client(),
			url,
			extra_headers,
			|err| ApiError::Http {
				status: err.status,
				message: err.message,
			},
		)
		.await
	}

	fn ensure_success(
		response: reqwest::Response
	) -> Result<reqwest::Response, ApiError> {
		if !response.status().is_success() {
			let status = response.status().as_u16();
			return Err(ApiError::http(status, format!("HTTP {}", status)));
		}
		Ok(response)
	}

	async fn fetch_json<T: serde::de::DeserializeOwned>(
		&self,
		url: &str,
		headers: Vec<(&'static str, String)>,
	) -> Result<T, ApiError> {
		let response = self.send_retried(url, headers).await?;
		let response = Self::ensure_success(response)?;
		response.json::<T>().await.map_err(Into::into)
	}

	/// Fetch JSON with conditional-GET support backed by the on-disk HTTP
	/// metadata cache (the process-global [`HttpMetaCache`]).
	///
	/// The flow:
	///
	/// 1. Look up `url` in the cache. If present and not stale, send
	///    `If-None-Match` / `If-Modified-Since` headers.
	/// 2. On `304 Not Modified`, deserialize the cached body and return.
	/// 3. On `200 OK`, capture the new validators and body, write them
	///    back to the cache, and return.
	/// 4. On any other status the regular error path applies.
	///
	/// Use this for **idempotent metadata endpoints** (project info,
	/// version lists) — never for binary downloads or write requests.
	///
	/// [`HttpMetaCache`]: crate::api::http_cache::HttpMetaCache
	async fn fetch_json_cached<T: serde::de::DeserializeOwned>(
		&self,
		url: &str,
		headers: Vec<(&'static str, String)>,
	) -> Result<T, ApiError> {
		crate::api::http_cache::conditional_fetch_json(
			self,
			crate::api::http_cache::HttpMetaCache::shared(),
			url,
			headers,
		)
		.await
	}
}

mod private {
	pub trait Sealed {}
}

macro_rules! define_api_client {
    // Variant with api_key support
    (
        $(#[$meta:meta])*
        $name:ident, $default_url:expr, api_key
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        pub struct $name {
            client: reqwest::Client,
            base_url: String,
            api_key: Option<String>,
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl crate::api::private::Sealed for $name {}

        impl crate::api::ApiClient for $name {
            fn http_client(&self) -> &reqwest::Client {
                &self.client
            }
        }

        impl $name {
            pub fn new() -> Self {
                Self {
                    client: crate::api::default_http_client(),
                    base_url: $default_url.to_string(),
                    api_key: None,
                }
            }

            #[must_use]
            #[allow(dead_code)] // used by tests + future test-server overrides
            pub fn with_client(mut self, client: reqwest::Client) -> Self {
                self.client = client;
                self
            }

            #[must_use]
            #[allow(dead_code)] // used by tests + future test-server overrides
            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.base_url = url.into();
                self
            }

            #[must_use]
            pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
                self.api_key = Some(key.into());
                self
            }

            pub fn has_api_key(&self) -> bool {
                self.api_key.is_some()
            }
        }
    };

    // Variant without api_key
    (
        $(#[$meta:meta])*
        $name:ident, $default_url:expr
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        pub struct $name {
            client: reqwest::Client,
            base_url: String,
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl crate::api::private::Sealed for $name {}

        impl crate::api::ApiClient for $name {
            fn http_client(&self) -> &reqwest::Client {
                &self.client
            }
        }

        impl $name {
            pub fn new() -> Self {
                Self {
                    client: crate::api::default_http_client(),
                    base_url: $default_url.to_string(),
                }
            }

            #[must_use]
            #[allow(dead_code)] // used by tests + future test-server overrides
            pub fn with_client(mut self, client: reqwest::Client) -> Self {
                self.client = client;
                self
            }

            #[must_use]
            #[allow(dead_code)] // used by tests + future test-server overrides
            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.base_url = url.into();
                self
            }
        }
    };
}

pub(crate) use define_api_client;
