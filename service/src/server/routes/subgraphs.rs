// Copyright 2023-, GraphOps and Semiotic Labs.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    extract::Extension,
    http::{self, Request, StatusCode},
    response::IntoResponse,
    Json,
};

use tracing::trace;

use crate::{
    query_processor::{FreeQuery, SubgraphDeploymentID},
    server::{
        routes::{bad_request_response, response_body_to_query_string},
        ServerOptions,
    },
};

/// Parse an incoming query request and route queries with authenticated
/// free query token to graph node
/// Later add receipt manager functions for paid queries
pub async fn subgraph_queries(
    Extension(server): Extension<ServerOptions>,
    id: axum::extract::Path<String>,
    req: Request<axum::body::Body>,
) -> impl IntoResponse {
    let (parts, body) = req.into_parts();

    // Extract scalar receipt from header and free query auth token for paid or free query
    let receipt = if let Some(receipt) = parts.headers.get("scalar-receipt") {
        match receipt.to_str() {
            Ok(r) => Some(r),
            Err(_) => {
                return bad_request_response("Bad scalar receipt for subgraph query");
            }
        }
    } else {
        None
    };
    trace!(
        "receipt attached by the query, can pass it to TAP: {:?}",
        receipt
    );

    // Extract free query auth token
    let auth_token = parts
        .headers
        .get(http::header::AUTHORIZATION)
        .and_then(|t| t.to_str().ok());
    // determine if the query is paid or authenticated to be free
    let free = auth_token.is_some()
        && server.free_query_auth_token.is_some()
        && auth_token.unwrap() == server.free_query_auth_token.as_deref().unwrap();

    let query_string = match response_body_to_query_string(body).await {
        Ok(q) => q,
        Err(e) => return bad_request_response(&e.to_string()),
    };

    // Initialize id into a subgraph deployment ID
    let subgraph_deployment_id = match SubgraphDeploymentID::new(id.as_str()) {
        Ok(id) => id,
        Err(e) => return bad_request_response(&e.to_string()),
    };

    if free {
        let free_query = FreeQuery {
            subgraph_deployment_id,
            query: query_string,
        };

        // TODO: Emit IndexerErrorCode::IE033 on error
        let res = server
            .query_processor
            .execute_free_query(free_query)
            .await
            .expect("Failed to execute free query");

        match res.status {
            200 => (StatusCode::OK, Json(res.result)).into_response(),
            _ => bad_request_response("Bad response from Graph node"),
        }
    } else if receipt.is_some() {
        let paid_query = crate::query_processor::PaidQuery {
            subgraph_deployment_id,
            query: query_string,
            receipt: receipt.unwrap().to_string(),
        };

        // TODO: Emit IndexerErrorCode::IE032 on error
        let res = server
            .query_processor
            .execute_paid_query(paid_query)
            .await
            .expect("Failed to execute paid query");

        match res.status {
            200 => (StatusCode::OK, Json(res.result)).into_response(),
            _ => bad_request_response("Bad response from Graph node"),
        }
    } else {
        // TODO: emit IndexerErrorCode::IE030 on missing receipt
        let error_body = "Query request header missing scalar-receipts or incorrect auth token";
        bad_request_response(error_body)
    }
}