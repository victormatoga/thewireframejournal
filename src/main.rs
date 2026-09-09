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
pub struct Article {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct CreateArticleForm {
    pub title: String,
    pub category: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct DeleteArticleForm {
    pub id: i64,
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
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:news.db?mode=rwc".to_string());
    let pool = SqlitePool::connect(&database_url).await?;

    // Execute SQL Table Initialization
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS articles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            category TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS admins (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL
        );"
    ).execute(&pool).await?;

    // Seed default admin user
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

    let tera = Tera::new("templates/**/*")?;
    let state = AppState { pool, tera };

    let app = Router::new()
        .route("/", get(|| async { Redirect::to("/admin/dashboard") }))
        .route("/admin/dashboard", get(admin_dashboard_handler))
        .route("/admin/articles/create", post(create_article_handler))
        .route("/admin/articles/delete", post(delete_article_handler))
        .route("/admin/change-password", post(change_admin_password_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 10000));
    println!("The Wireframe Journal active on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

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

    let articles = sqlx::query_as::<_, Article>("SELECT id, title, category, content FROM articles ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("metrics", &metrics);
    ctx.insert("articles", &articles);
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

async fn create_article_handler(
    State(state): State<AppState>,
    Form(form): Form<CreateArticleForm>,
) -> impl IntoResponse {
    let result = sqlx::query("INSERT INTO articles (title, category, content) VALUES ($1, $2, $3)")
        .bind(&form.title)
        .bind(&form.category)
        .bind(&form.content)
        .execute(&state.pool)
        .await;

    match result {
        Ok(_) => Redirect::to("/admin/dashboard?msg=Article+published+successfully!"),
        Err(_) => Redirect::to("/admin/dashboard?error=Failed+to+publish+article"),
    }
}

async fn delete_article_handler(
    State(state): State<AppState>,
    Form(form): Form<DeleteArticleForm>,
) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM articles WHERE id = $1")
        .bind(form.id)
        .execute(&state.pool)
        .await;

    match result {
        Ok(_) => Redirect::to("/admin/dashboard?msg=Article+deleted+successfully!"),
        Err(_) => Redirect::to("/admin/dashboard?error=Failed+to+delete+article"),
    }
}

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