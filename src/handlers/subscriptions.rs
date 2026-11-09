//! Subscription endpoints.
//!
//! Behavioral notes carried over from `controllers/subscription.go`:
//! - Q19: the lookup excludes soft-deleted rows, so re-subscribing a
//!   soft-deleted address hits the unique email index and returns 500
//!   `Failed to subscribe`; this is intentionally not an upsert.
//! - Re-subscribing an inactive row rotates its token, sends no email and
//!   returns 200; a brand-new row sends the confirmation email.
//! - Q9 (fix): notifications are sent with `buffer_unordered(5)` and failures
//!   are counted/logged, while `count` keeps Go's "active subscribers" meaning.
//! - Q10: unsubscribe links are built from `site_url` (the default produces
//!   the same bytes as the frozen Go fixture).

use futures::StreamExt;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{MessageResponse, NotifyResponse, SubscriptionDto, SubscriptionListResponse};
use crate::repo::subscriptions;
use crate::services::email;
use crate::services::params::{gorm_pagination, parse_digits_prefix};
use crate::services::token;
use crate::services::validation;
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
struct SubscribeRequest {
    email: String,
}

/// `POST /api/subscribe`
pub async fn subscribe(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<MessageResponse>, AppError> {
    let request: SubscribeRequest = parse_body(&body, "Invalid email")?;
    if request.email.is_empty() || !validation::is_valid_email(&request.email) {
        return Err(AppError::BadRequest("Invalid email"));
    }

    // Go treats any lookup error as "not found" and proceeds to the insert.
    let existing = match subscriptions::find_by_email(&state.db, &request.email).await {
        Ok(existing) => existing,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to look up subscription");
            None
        }
    };

    if let Some(existing) = existing {
        if existing.is_active.unwrap_or(false) {
            return Err(AppError::BadRequest("Already subscribed"));
        }

        // Re-activate with a fresh token; Go ignores the Save error and does
        // not send another confirmation email.
        let new_token = token::generate_unsubscribe_token();
        if let Err(err) = subscriptions::reactivate(&state.db, existing.id, &new_token).await {
            tracing::error!(
                error = ?err,
                subscription_id = existing.id,
                "Failed to reactivate subscription"
            );
        }
        return Ok(Json(MessageResponse {
            message: "Subscribed successfully",
        }));
    }

    let new_token = token::generate_unsubscribe_token();
    subscriptions::create(&state.db, &request.email, &new_token)
        .await
        .map_err(|err| AppError::internal_msg("Failed to subscribe", err))?;

    let (subject, body) =
        email::subscribe_confirmation_email(&state.cfg.site_url, &new_token, &request.email);
    // Go discards the send error; the subscription is already stored.
    if let Err(err) = state.mailer.send(&request.email, &subject, &body).await {
        tracing::error!(
            error = ?err,
            recipient = %request.email,
            "Failed to send subscription confirmation"
        );
    }

    Ok(Json(MessageResponse {
        message: "Subscribed successfully",
    }))
}

#[derive(Debug, Default, Deserialize)]
pub struct UnsubscribeQuery {
    token: Option<String>,
    email: Option<String>,
}

/// `GET /api/unsubscribe`
pub async fn unsubscribe(
    State(state): State<SharedState>,
    Query(query): Query<UnsubscribeQuery>,
) -> Result<Json<MessageResponse>, AppError> {
    let email = query.email.unwrap_or_default();
    if email.is_empty() {
        return Err(AppError::BadRequest("Invalid parameters"));
    }
    let token = query.token.unwrap_or_default();

    // A lookup error is reported as "not found", exactly like Go treating any
    // `First` error as a 404.
    let found = if token.is_empty() {
        subscriptions::find_by_email(&state.db, &email).await
    } else {
        subscriptions::find_by_email_and_token(&state.db, &email, &token).await
    };
    let Some(subscription) = found.ok().flatten() else {
        return Err(AppError::NotFound("Subscription not found"));
    };

    // Go ignores the Save error and still returns 200.
    if let Err(err) = subscriptions::set_active(&state.db, subscription.id, false).await {
        tracing::error!(
            error = ?err,
            subscription_id = subscription.id,
            "Failed to unsubscribe"
        );
    }

    Ok(Json(MessageResponse {
        message: "Unsubscribed successfully",
    }))
}

#[derive(Debug, Default, Deserialize)]
pub struct SubscriptionListQuery {
    page: Option<String>,
    page_size: Option<String>,
}

/// `GET /api/admin/subscriptions`
pub async fn get_subscriptions(
    State(state): State<SharedState>,
    Query(query): Query<SubscriptionListQuery>,
) -> Result<Json<SubscriptionListResponse>, AppError> {
    // Q13: Go's subscription handler uses the digit-skipping `parseInt`.
    let page = query.page.as_deref().map(parse_digits_prefix).unwrap_or(1);
    let page_size = query
        .page_size
        .as_deref()
        .map(parse_digits_prefix)
        .unwrap_or(20);
    let (limit, offset) = gorm_pagination(page, page_size);

    let total = subscriptions::count(&state.db).await.map_err(internal)?;
    let rows = subscriptions::list(&state.db, limit, offset)
        .await
        .map_err(internal)?;

    Ok(Json(SubscriptionListResponse {
        list: rows
            .into_iter()
            .map(|row| SubscriptionDto::from_row(row, state.tz))
            .collect(),
        total,
    }))
}

/// `DELETE /api/admin/subscriptions/_id/:id`
pub async fn delete_subscription(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    subscriptions::soft_delete(&state.db, &raw_id)
        .await
        .map_err(|err| AppError::internal_msg("Failed to delete subscription", err))?;
    Ok(Json(MessageResponse {
        message: "Subscription deleted",
    }))
}

#[derive(Debug, Deserialize)]
struct NotifyRequest {
    post_id: u64,
}

/// `POST /api/admin/notify`
pub async fn notify_subscribers(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<NotifyResponse>, AppError> {
    let request: NotifyRequest = parse_body(&body, "Invalid request")?;
    if request.post_id == 0 {
        return Err(AppError::BadRequest("Invalid request"));
    }

    let Some(post) = crate::repo::posts::find_by_id(&state.db, request.post_id)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Post not found"));
    };

    // Go ignores the query error (it would simply send nothing).
    let subscribers = subscriptions::list_active(&state.db)
        .await
        .unwrap_or_else(|err| {
            tracing::error!(error = ?err, "Failed to load active subscriptions");
            Vec::new()
        });

    let count = subscribers.len();
    let mailer = state.mailer.clone();
    let site_url = state.cfg.site_url.clone();
    let title = post.title;
    let excerpt = post.excerpt.unwrap_or_default();
    let slug = post.slug;

    // Q9: at most five SMTP sends in flight; failures are collected instead of
    // being silently lost.
    let sends = subscribers.into_iter().map(|subscriber| {
        let mailer = mailer.clone();
        let site_url = site_url.clone();
        let title = title.clone();
        let excerpt = excerpt.clone();
        let slug = slug.clone();
        async move {
            let (subject, body) = email::new_post_notification_email(
                &site_url,
                &title,
                &excerpt,
                &slug,
                subscriber.token.as_deref().unwrap_or_default(),
                &subscriber.email,
            );
            let outcome = mailer.send(&subscriber.email, &subject, &body).await;
            (subscriber.email, outcome)
        }
    });
    let results: Vec<(String, anyhow::Result<()>)> = futures::stream::iter(sends)
        .buffer_unordered(5)
        .collect()
        .await;

    let failed = results
        .iter()
        .filter(|(_, outcome)| outcome.is_err())
        .count();
    for (recipient, outcome) in &results {
        if let Err(err) = outcome {
            tracing::warn!(
                error = ?err,
                recipient = %recipient,
                "Failed to send notification email"
            );
        }
    }
    tracing::info!(
        sent = count.saturating_sub(failed),
        failed,
        total = count,
        "Subscriber notifications completed"
    );

    Ok(Json(NotifyResponse {
        count,
        message: "Notifications sent",
    }))
}

fn parse_body<T: serde::de::DeserializeOwned>(
    body: &Result<Bytes, BytesRejection>,
    message: &'static str,
) -> Result<T, AppError> {
    let bytes = body.as_ref().map_err(|_| AppError::BadRequest(message))?;
    serde_json::from_slice(bytes).map_err(|_| AppError::BadRequest(message))
}

fn internal(err: sqlx::Error) -> AppError {
    AppError::internal_msg("Internal server error", err)
}
