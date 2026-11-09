//! Smileyan Blog backend — Rust rewrite.
//!
//! Phase 1 provides the scaffold (configuration, logging, database/Redis,
//! error handling, HTTP router). Phase 2 adds authentication, users and avatar
//! upload. Phase 3 adds posts, categories, tags, pages and the Markdown
//! pipeline. Phase 4 adds comments, subscriptions and email notifications.
//! Phase 5 adds the in-memory tantivy search index and `/api/search`. Phase 6
//! adds the TTS proxy, moves avatar upload to `handlers::uploads`, and serves
//! `/uploads`; later phases cover contract/performance/security and deployment.

pub mod app;
pub mod bootstrap;
pub mod config;
pub mod error;
pub mod handlers;
pub mod logging;
pub mod middleware;
pub mod models;
pub mod repo;
pub mod services;
pub mod state;
