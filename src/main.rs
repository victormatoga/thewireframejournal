use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Form, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::net::SocketAddr;
use tera::Tera;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub tera: Tera,
}

#[derive(Serialize)]
pub struct DashboardMetrics {
    pub total_views: i64,
    pub total_subscribers: i64,
    pub total_revenue: f64,
    pub total_clicks: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Subscriber {
    pub id: i64,
    pub email: String,
    pub status: Option<String>,
    pub subscribed_at: Option<String>,
}

#[derive(Deserialize)]
pub struct PasswordChangeForm {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct DashboardQuery {
    pub msg: Option<String>,
    pub error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Database Connection Setup
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:news.db?mode=rwc".to_string());
    let pool = SqlitePool::connect(&database_url).await?;

    // Execute Schema Setup
    let schema = std::fs::read_to_string("init_db.sql")
        .unwrap_or_else(|_| include_str!("../init_db.sql").to_string());
    sqlx::query(&schema).execute(&pool).await?;

    // Seed default admin user if missing (default: admin / adminpassword)
    let admin_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    if admin_exists == 0 {
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(b"adminpassword", &salt)
            .expect("Failed to hash default admin password")
            .to_string();

        sqlx::query("INSERT INTO admins (username, password_hash) VALUES ($1, $2)")
            .bind("admin")
            .bind(password_hash)
            .execute(&pool)
            .await?;
    }

    // Initialize Tera Templates
    let tera = Tera::new("templates/**/*")?;
    let state = AppState { pool, tera };

    // Application Routes (Added "/" root redirect to dashboard)
    let app = Router::new()
        .route("/", get(|| async { Redirect::to("/admin/dashboard") }))
        .route("/admin/dashboard", get(admin_dashboard_handler))
        .route("/admin/change-password", post(change_admin_password_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 10000));
    println!("Server live on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// Handler: Render Admin Dashboard
async fn admin_dashboard_handler(
    State(state): State<AppState>,
    Query(params): Query<DashboardQuery>,
) -> impl IntoResponse {
    let views: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM article_views")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    let subs_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subscribers WHERE status = 'active'")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    let revenue: Option<f64> = sqlx::query_scalar("SELECT SUM(estimated_revenue) FROM ad_impressions")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(None);

    let clicks: Option<i64> = sqlx::query_scalar("SELECT SUM(clicks) FROM ad_impressions")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(None);

    let metrics = DashboardMetrics {
        total_views: views,
        total_subscribers: subs_count,
        total_revenue: revenue.unwrap_or(0.0),
        total_clicks: clicks.unwrap_or(0),
    };

    let subscribers = sqlx::query_as::<_, Subscriber>(
        "SELECT id, email, status, datetime(subscribed_at) as subscribed_at FROM subscribers ORDER BY id DESC LIMIT 10"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("metrics", &metrics);
    ctx.insert("subscribers", &subscribers);
    if let Some(m) = params.msg { ctx.insert("msg", &m); }
    if let Some(e) = params.error { ctx.insert("error", &e); }

    match state.tera.render("admin_dashboard.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template Error: {}", err),
        )
            .into_response(),
    }
}

// Handler: Update Admin Password with Argon2
async fn change_admin_password_handler(
    State(state): State<AppState>,
    Form(form): Form<PasswordChangeForm>,
) -> impl IntoResponse {
    let admin_row = match sqlx::query("SELECT id, password_hash FROM admins WHERE username = $1")
        .bind("admin")
        .fetch_one(&state.pool)
        .await
    {
        Ok(row) => row,
        Err(_) => return Redirect::to("/admin/dashboard?error=Admin+user+not+found"),
    };

    let admin_id: i64 = admin_row.get("id");
    let stored_hash: String = admin_row.get("password_hash");

    let parsed_hash = match PasswordHash::new(&stored_hash) {
        Ok(hash) => hash,
        Err(_) => return Redirect::to("/admin/dashboard?error=Invalid+stored+password+hash"),
    };

    if Argon2::default()
        .verify_password(form.current_password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return Redirect::to("/admin/dashboard?error=Incorrect+current+password");
    }

    let salt = SaltString::generate(&mut OsRng);
    let new_hash = match Argon2::default().hash_password(form.new_password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(_) => return Redirect::to("/admin/dashboard?error=Failed+to+hash+new+password"),
    };

    if sqlx::query("UPDATE admins SET password_hash = $1 WHERE id = $2")
        .bind(new_hash)
        .bind(admin_id)
        .execute(&state.pool)
        .await
        .is_err()
    {
        return Redirect::to("/admin/dashboard?error=Failed+to+update+database");
    }

    Redirect::to("/admin/dashboard?msg=Password+updated+successfully!")
}