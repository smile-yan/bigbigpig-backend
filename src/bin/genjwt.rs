//! Replacement for `go run cmd/genjwt`: prints a dev token and an admin token.
//!
//! Defaults match `cmd/genjwt/main.go`; `SMILEYAN_BACKEND_JWT_SECRET` and
//! `SMILEYAN_BACKEND_DEV_TOKEN` override them.

use std::env;

use smileyan_backend::services::token::{generate_dev_token, generate_genjwt_admin_token};

fn main() -> anyhow::Result<()> {
    let secret = env::var("SMILEYAN_BACKEND_JWT_SECRET")
        .unwrap_or_else(|_| "smileyan-jwt-secret-key-2024".to_string());
    let dev_value = env::var("SMILEYAN_BACKEND_DEV_TOKEN")
        .unwrap_or_else(|_| "dev-skip-auth-token-2024".to_string());

    let one_year = 365 * 24 * 3600;
    let dev_token = generate_dev_token(&secret, &dev_value, one_year)?;
    let admin_token = generate_genjwt_admin_token(&secret, "i@ccyan.cn", one_year)?;

    println!("=== 方式1: Dev Token（需服务器设置 DEV_TOKEN 环境变量）===");
    println!("{dev_token}");
    println!();
    println!("=== 方式2: 管理员 Token（直接声明 admin 角色，推荐）===");
    println!("{admin_token}");
    println!();
    println!("使用方法：");
    println!(
        "  curl -H 'Authorization: Bearer {admin_token}' https://bigbigpig.cn/api/admin/posts"
    );

    Ok(())
}
