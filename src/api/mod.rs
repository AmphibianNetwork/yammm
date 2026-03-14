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
pub mod installer;
pub mod loader;
pub mod minecraft;
pub mod modrinth;
pub mod neoforge;
pub mod retry;

pub use curseforge::{CfFile, CfProject, CurseForgeClient};
pub use error::ApiError;
pub use forge::ForgeClient;
pub use github::{GitHubAsset, GitHubClient, GitHubRelease};
pub use installer::{InstallProfile, LoaderInstallResult};
pub use loader::{
	FabricClient, FabricLibrary, FabricProfile, QuiltClient, QuiltProfile,
};
pub use minecraft::MinecraftClient;
pub use modrinth::{ModrinthClient, ModrinthSearchHit, ModrinthVersion};
pub use neoforge::NeoForgeClient;

/// Shared HTTP helpers for all API clients.
#[allow(async_fn_in_trait)]
pub trait ApiClient: private::Sealed {
	fn http_client(&self) -> &reqwest::Client;

	async fn send_retried(
		&self,
		url: &str,
		extra_headers: Vec<(String, String)>,
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
		headers: Vec<(String, String)>,
	) -> Result<T, ApiError> {
		let response = self.send_retried(url, headers).await?;
		let response = Self::ensure_success(response)?;
		response.json::<T>().await.map_err(Into::into)
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
                    client: reqwest::Client::new(),
                    base_url: $default_url.to_string(),
                    api_key: None,
                }
            }

            pub fn with_client(mut self, client: reqwest::Client) -> Self {
                self.client = client;
                self
            }

            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.base_url = url.into();
                self
            }

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
                    client: reqwest::Client::new(),
                    base_url: $default_url.to_string(),
                }
            }

            pub fn with_client(mut self, client: reqwest::Client) -> Self {
                self.client = client;
                self
            }

            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.base_url = url.into();
                self
            }
        }
    };
}

pub(crate) use define_api_client;
