use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use log::info;
use openidconnect::AccessToken;
use openidconnect::AccessTokenHash;
use openidconnect::EndpointMaybeSet;
use openidconnect::EndpointNotSet;
use openidconnect::EndpointSet;
use openidconnect::NonceVerifier;
use openidconnect::OAuth2TokenResponse;
use openidconnect::RefreshToken;
use openidconnect::core::CoreIdToken;
use openidconnect::core::CoreIdTokenClaims;
use openidconnect::core::CoreTokenResponse;
use openidconnect::{
    AdditionalProviderMetadata, AuthType, ClientId, DeviceAuthorizationUrl, IssuerUrl,
    ProviderMetadata,
    core::{
        CoreAuthDisplay, CoreClaimName, CoreClaimType, CoreClient, CoreClientAuthMethod,
        CoreDeviceAuthorizationResponse, CoreGrantType, CoreJsonWebKey,
        CoreJweContentEncryptionAlgorithm, CoreJweKeyManagementAlgorithm, CoreResponseMode,
        CoreResponseType, CoreSubjectIdentifierType,
    },
};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use std::env;
use std::str::FromStr;

use crate::AuthCommands;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceEndpointProviderMetadata {
    device_authorization_endpoint: DeviceAuthorizationUrl,
}
impl AdditionalProviderMetadata for DeviceEndpointProviderMetadata {}
type DeviceProviderMetadata = ProviderMetadata<
    DeviceEndpointProviderMetadata,
    CoreAuthDisplay,
    CoreClientAuthMethod,
    CoreClaimName,
    CoreClaimType,
    CoreGrantType,
    CoreJweContentEncryptionAlgorithm,
    CoreJweKeyManagementAlgorithm,
    CoreJsonWebKey,
    CoreResponseMode,
    CoreResponseType,
    CoreSubjectIdentifierType,
>;

/// Device code auth does not support nonces
struct DummyNonce;

impl NonceVerifier for DummyNonce {
    fn verify(self, _nonce: Option<&openidconnect::Nonce>) -> Result<(), String> {
        Ok(())
    }
}

#[allow(dead_code)]
pub trait TokenStore {
    fn set_access_token(&self, token: &str) -> Result<()>;
    fn set_refresh_token(&self, token: &str) -> Result<()>;
    fn set_id_token(&self, token: &str) -> Result<()>;
    fn get_access_token(&self) -> Result<String>;
    fn get_refresh_token(&self) -> Result<String>;
    fn get_id_token(&self) -> Result<String>;
    fn logout(&self) -> Result<()>;
}

pub type DiscoveredClient = CoreClient<
    EndpointSet,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub struct AuthState {
    store: Box<dyn TokenStore>,
    client: DiscoveredClient,
    http_client: reqwest::blocking::Client,
}

impl AuthState {
    pub fn new(
        store: impl TokenStore + 'static,
        provider_metadata: DeviceProviderMetadata,
        client_id: ClientId,
    ) -> Result<Self> {
        let device_endpoint = provider_metadata
            .additional_metadata()
            .device_authorization_endpoint
            .clone();

        Ok(Self {
            store: Box::new(store),
            client: CoreClient::from_provider_metadata(provider_metadata, client_id, None)
                .set_device_authorization_url(device_endpoint)
                .set_auth_type(AuthType::RequestBody),
            http_client: reqwest::blocking::ClientBuilder::new()
                .redirect(Policy::none())
                .build()?,
        })
    }

    pub fn update_from_response(
        &self,
        token: &CoreTokenResponse,
    ) -> Result<(
        AccessToken,
        Option<RefreshToken>,
        CoreIdToken,
        CoreIdTokenClaims,
    )> {
        let id_token = token
            .extra_fields()
            .id_token()
            .ok_or_else(|| anyhow!("Server did not return an ID token"))?;

        let claims = id_token.claims(&self.client.id_token_verifier(), DummyNonce)?;

        // Verify access token hash
        if let Some(expected_access_token_hash) = claims.access_token_hash() {
            let actual_access_token_hash = AccessTokenHash::from_token(
                token.access_token(),
                id_token.signing_alg()?,
                id_token.signing_key(&self.client.id_token_verifier())?,
            )?;
            if actual_access_token_hash != *expected_access_token_hash {
                return Err(anyhow!("Invalid access token"));
            }
        }

        let access_token = token.access_token();
        let refresh_token = token.refresh_token();

        self.store.set_access_token(access_token.secret())?;
        if let Some(refresh_token) = refresh_token {
            self.store.set_refresh_token(refresh_token.secret())?;
        }
        self.store.set_id_token(&id_token.to_string())?;

        Ok((
            access_token.clone(),
            refresh_token.cloned(),
            id_token.clone(),
            claims.clone(),
        ))
    }

    /// Get ID token claims, transparently refreshing it
    pub fn id_token_claims(&self) -> Result<CoreIdTokenClaims> {
        let id_token = CoreIdToken::from_str(&self.store.get_id_token()?)?;

        let claims = id_token.claims(&self.client.id_token_verifier(), DummyNonce);

        let claims = match claims {
            Ok(claims) => claims,
            Err(openidconnect::ClaimsVerificationError::Expired(_)) => {
                info!("Token expired refreshing...");
                let refresh_token = RefreshToken::new(self.store.get_refresh_token()?);
                let response = &self
                    .client
                    .exchange_refresh_token(&refresh_token)?
                    .request(&self.http_client)?;

                &self.update_from_response(response)?.3
            }
            e => e?,
        };

        Ok(claims.clone())
    }
}

pub struct KeyringTokenStore;

impl KeyringTokenStore {
    fn access_entry() -> Result<keyring::Entry> {
        keyring::Entry::new("machine-locks-client:access-token", "default").map_err(Into::into)
    }
    fn refresh_entry() -> Result<keyring::Entry> {
        keyring::Entry::new("machine-locks-client:refresh-token", "default").map_err(Into::into)
    }
    fn id_entry() -> Result<keyring::Entry> {
        keyring::Entry::new("machine-locks-client:id-token", "default").map_err(Into::into)
    }
}

impl TokenStore for KeyringTokenStore {
    fn set_access_token(&self, token: &str) -> Result<()> {
        KeyringTokenStore::access_entry()?
            .set_password(token)
            .map_err(Into::into)
    }

    fn set_refresh_token(&self, token: &str) -> Result<()> {
        KeyringTokenStore::refresh_entry()?
            .set_password(token)
            .map_err(Into::into)
    }

    fn set_id_token(&self, token: &str) -> Result<()> {
        KeyringTokenStore::id_entry()?
            .set_password(token)
            .map_err(Into::into)
    }

    fn get_access_token(&self) -> Result<String> {
        KeyringTokenStore::access_entry()?.get_password().context(
            "Failed to get access token. Run `machine-locks-client auth login` to login again",
        )
    }

    fn get_refresh_token(&self) -> Result<String> {
        KeyringTokenStore::refresh_entry()?.get_password().context(
            "Failed to get refresh token. Run `machine-locks-client auth login` to login again",
        )
    }

    fn get_id_token(&self) -> Result<String> {
        KeyringTokenStore::id_entry()?
            .get_password()
            .context("Failed to get ID token. Run `machine-locks-client auth login` to login again")
    }

    fn logout(&self) -> Result<()> {
        KeyringTokenStore::access_entry()?.delete_credential()?;
        KeyringTokenStore::refresh_entry()?.delete_credential()?;
        KeyringTokenStore::id_entry()?.delete_credential()?;

        Ok(())
    }
}

fn print_claims(claims: &CoreIdTokenClaims) {
    println!(
        "Authenticated as: {}\nsubject: {}.\nexpiry: {}",
        claims
            .preferred_username()
            .map(|e| e.to_string())
            .as_deref()
            .unwrap_or("<username not set>"),
        claims.subject().to_string(),
        claims.expiration()
    );
}

fn login(state: AuthState) -> Result<()> {
    println!("Fetching device code...");
    let details: CoreDeviceAuthorizationResponse = state
        .client
        .exchange_device_code()
        .request(&state.http_client)?;

    println!(
        "Open this URL in your browser:\n{}\nand enter the code: {}",
        details.verification_uri_complete().unwrap().secret(),
        details.user_code().secret()
    );

    let response = state
        .client
        .exchange_device_access_token(&details)?
        .request(&state.http_client, std::thread::sleep, None)?;

    let claims = state.update_from_response(&response)?.3;

    print_claims(&claims);

    Ok(())
}

fn status(state: AuthState) -> Result<()> {
    print_claims(&state.id_token_claims()?);
    Ok(())
}

fn logout() -> Result<()> {
    let token_store = KeyringTokenStore;
    token_store.logout()
}

pub fn auth_commands(command: AuthCommands) -> Result<()> {
    let issuer_url = env::var("MACHINE_LOCKS_ISSUER_URL")
        .unwrap_or("https://sso.trustworthy.systems/realms/trustworthy_systems".to_string());

    let client_id = ClientId::new(env::var("MACHINE_LOCKS_CLIENT_ID").unwrap_or("mq".to_string()));

    // Not connecting to machine-locks-server API, so dedicated
    // client that does not follow redirects
    let http_client = reqwest::blocking::ClientBuilder::new()
        .redirect(Policy::none())
        .build()?;

    let provider_metadata =
        DeviceProviderMetadata::discover(&IssuerUrl::new(issuer_url)?, &http_client)?;

    let auth_state = AuthState::new(KeyringTokenStore, provider_metadata, client_id)?;

    match command {
        AuthCommands::Login => login(auth_state)?,
        AuthCommands::Status => status(auth_state)?,
        AuthCommands::Logout => logout()?,
    }
    Ok(())
}
