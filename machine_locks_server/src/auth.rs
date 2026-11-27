// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause

use std::str::FromStr;

use axum::{
    extract::{Request, State},
    http::{self, HeaderName, StatusCode},
    middleware::Next,
    response::IntoResponse,
};
use common::DummyNonce;
use log::warn;
use openidconnect::{
    ClientId, IdTokenVerifier, IssuerUrl, JsonWebKeySet,
    core::{CoreIdToken, CoreIdTokenClaims},
};

use crate::AppState;

#[derive(Debug, Clone)]
pub struct AuthContext(CoreIdTokenClaims);

/// Authentication middleware
pub async fn layer(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> impl IntoResponse {
    let headers = request.headers();
    let auth_header = headers
        .get(http::header::AUTHORIZATION)
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = auth_header
        .to_str()
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::BAD_REQUEST)?;

    let client_id = ClientId::new(
        headers
            .get(HeaderName::from_static("x-auth-client-id"))
            .ok_or(StatusCode::BAD_REQUEST)?
            .to_str()
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .to_owned(),
    );

    let issuer = IssuerUrl::new(
        headers
            .get(HeaderName::from_static("x-auth-issuer"))
            .ok_or(StatusCode::BAD_REQUEST)?
            .to_str()
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .to_owned(),
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;

    // Validate client ID and issuer
    if !state.client_ids.contains(&client_id) || !state.issuer_urls.contains(&issuer) {
        warn!(
            "Invalid client ID or issuer_url, client {:?} issuer {:?}",
            client_id, issuer
        );
        return Err(StatusCode::FORBIDDEN);
    }

    let keys = JsonWebKeySet::new(state.keys);
    let verifier = IdTokenVerifier::new_public_client(client_id, issuer, keys);

    let token = CoreIdToken::from_str(token).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let claims = token
        .claims(&verifier, DummyNonce)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    dbg!(claims);

    request.extensions_mut().insert(AuthContext(claims.clone()));

    Ok(next.run(request).await)
}
