use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Path, Query, State},
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

#[derive(Serialize, sqlx::FromRow)]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub content: String,
    pub author_role: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateArticleForm {
    pub title: String,
    pub category: String,
    pub content: String,
    pub role: String,
}

#[derive(Deserialize)]
pub struct DeleteArticleForm {
    pub id: i64,
    pub role: String,
}

#[derive(Deserialize)]
pub struct SubscribeForm {
    pub email: String,
}

#[derive(Deserialize)]
pub struct PasswordChangeForm {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct AuthQuery {
    pub role: Option<String>,
    pub msg: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub msg: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:news.db?mode=rwc".to_string());
    let pool = SqlitePool::connect(&database_url).await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS articles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            category TEXT NOT NULL,
            content TEXT NOT NULL,
            author_role TEXT DEFAULT 'reporter',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS admins (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT DEFAULT 'super_admin'
        );
        CREATE TABLE IF NOT EXISTS subscribers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT UNIQUE NOT NULL,
            subscribed_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );"
    ).execute(&pool).await?;

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

        sqlx::query("INSERT INTO admins (username, password_hash, role) VALUES ($1, $2, $3)")
            .bind("admin")
            .bind(password_hash)
            .bind("super_admin")
            .execute(&pool)
            .await?;
    }

    let tera = Tera::new("templates/**/*")?;
    let state = AppState { pool, tera };

    let app = Router::new()
        .route("/", get(public_website_handler))
        .route("/category/:name", get(public_category_handler))
        .route("/search", get(public_search_handler))
        .route("/subscribe", post(subscribe_handler))
        .route("/admin/dashboard", get(admin_dashboard_handler))
        .route("/admin/editor", get(editor_portal_handler))
        .route("/admin/reporter", get(reporter_portal_handler))
        .route("/admin/articles/create", post(create_article_handler))
        .route("/admin/articles/delete", post(delete_article_handler))
        .route("/admin/change-password", post(change_admin_password_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 10000));
    println!("The WireFrame Journal server running on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn get_categories() -> Vec<&'static str> {
    vec![
        "breaking news", "international", "sports", "healthy", "agriculture",
        "weather", "climate", "politics", "games", "entertainment", "business"
    ]
}

async fn public_website_handler(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let articles = sqlx::query_as::<_, Article>("SELECT id, title, category, content, author_role FROM articles ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("categories", &get_categories());
    ctx.insert("articles", &articles);
    ctx.insert("active_category", "");
    ctx.insert("search_term", "");
    if let Some(m) = params.msg { ctx.insert("msg", &m); }

    match state.tera.render("index.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", err)).into_response(),
    }
}

async fn public_category_handler(
    State(state): State<AppState>,
    Path(category): Path<String>,
) -> impl IntoResponse {
    let articles = sqlx::query_as::<_, Article>("SELECT id, title, category, content, author_role FROM articles WHERE category = $1 ORDER BY id DESC")
        .bind(&category)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("categories", &get_categories());
    ctx.insert("active_category", &category);
    ctx.insert("search_term", "");
    ctx.insert("articles", &articles);

    match state.tera.render("index.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", err)).into_response(),
    }
}

async fn public_search_handler(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let search_term = query.q.unwrap_or_default();
    let pattern = format!("%{}%", search_term);

    let articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, category, content, author_role FROM articles WHERE title LIKE $1 OR content LIKE $2 ORDER BY id DESC"
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("categories", &get_categories());
    ctx.insert("search_term", &search_term);
    ctx.insert("active_category", "");
    ctx.insert("articles", &articles);

    match state.tera.render("index.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", err)).into_response(),
    }
}

async fn subscribe_handler(
    State(state): State<AppState>,
    Form(form): Form<SubscribeForm>,
) -> impl IntoResponse {
    let result = sqlx::query("INSERT INTO subscribers (email) VALUES ($1)")
        .bind(&form.email)
        .execute(&state.pool)
        .await;

    match result {
        Ok(_) => Redirect::to("/?msg=Thank+you+for+subscribing+to+The+WireFrame+Journal!"),
        Err(_) => Redirect::to("/?msg=This+email+is+already+subscribed."),
    }
}

async fn admin_dashboard_handler(
    State(state): State<AppState>,
    Query(params): Query<AuthQuery>,
) -> impl IntoResponse {
    let articles = sqlx::query_as::<_, Article>("SELECT id, title, category, content, author_role FROM articles ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("articles", &articles);
    ctx.insert("categories", &get_categories());
    ctx.insert("user_role", "Super Admin");
    if let Some(m) = params.msg { ctx.insert("msg", &m); }
    if let Some(e) = params.error { ctx.insert("error", &e); }

    match state.tera.render("admin_dashboard.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", err)).into_response(),
    }
}

async fn editor_portal_handler(
    State(state): State<AppState>,
    Query(params): Query<AuthQuery>,
) -> impl IntoResponse {
    let articles = sqlx::query_as::<_, Article>("SELECT id, title, category, content, author_role FROM articles ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("articles", &articles);
    ctx.insert("categories", &get_categories());
    ctx.insert("user_role", "Editor");
    if let Some(m) = params.msg { ctx.insert("msg", &m); }
    if let Some(e) = params.error { ctx.insert("error", &e); }

    match state.tera.render("editor_portal.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", err)).into_response(),
    }
}

async fn reporter_portal_handler(
    State(state): State<AppState>,
    Query(params): Query<AuthQuery>,
) -> impl IntoResponse {
    let articles = sqlx::query_as::<_, Article>("SELECT id, title, category, content, author_role FROM articles WHERE author_role = 'reporter' ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let mut ctx = tera::Context::new();
    ctx.insert("articles", &articles);
    ctx.insert("categories", &get_categories());
    ctx.insert("user_role", "Reporter");
    if let Some(m) = params.msg { ctx.insert("msg", &m); }
    if let Some(e) = params.error { ctx.insert("error", &e); }

    match state.tera.render("reporter_portal.html", &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", err)).into_response(),
    }
}

async fn create_article_handler(
    State(state): State<AppState>,
    Form(form): Form<CreateArticleForm>,
) -> impl IntoResponse {
    let result = sqlx::query("INSERT INTO articles (title, category, content, author_role) VALUES ($1, $2, $3, $4)")
        .bind(&form.title)
        .bind(&form.category)
        .bind(&form.content)
        .bind(&form.role)
        .execute(&state.pool)
        .await;

    let redirect_url = match form.role.as_str() {
        "editor" => "/admin/editor?msg=Article+published!",
        "reporter" => "/admin/reporter?msg=Draft+submitted!",
        _ => "/admin/dashboard?msg=Article+published!",
    };

    match result {
        Ok(_) => Redirect::to(redirect_url),
        Err(_) => Redirect::to("/admin/dashboard?error=Failed+to+create+article"),
    }
}

async fn delete_article_handler(
    State(state): State<AppState>,
    Form(form): Form<DeleteArticleForm>,
) -> impl IntoResponse {
    if form.role == "reporter" {
        return Redirect::to("/admin/reporter?error=Reporters+cannot+delete+articles");
    }

    let result = sqlx::query("DELETE FROM articles WHERE id = $1")
        .bind(form.id)
        .execute(&state.pool)
        .await;

    let redirect_url = if form.role == "editor" {
        "/admin/editor?msg=Article+deleted"
    } else {
        "/admin/dashboard?msg=Article+deleted"
    };

    match result {
        Ok(_) => Redirect::to(redirect_url),
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
        Err(_) => return Redirect::to("/admin/dashboard?error=Admin+not+found"),
    };

    let admin_id: i64 = admin_row.get("id");
    let stored_hash: String = admin_row.get("password_hash");

    let parsed_hash = match PasswordHash::new(&stored_hash) {
        Ok(hash) => hash,
        Err(_) => return Redirect::to("/admin/dashboard?error=Invalid+hash"),
    };

    if Argon2::default().verify_password(form.current_password.as_bytes(), &parsed_hash).is_err() {
        return Redirect::to("/admin/dashboard?error=Incorrect+password");
    }

    let salt = SaltString::generate(&mut OsRng);
    let new_hash = match Argon2::default().hash_password(form.new_password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(_) => return Redirect::to("/admin/dashboard?error=Failed+to+hash"),
    };

    if sqlx::query("UPDATE admins SET password_hash = $1 WHERE id = $2")
        .bind(new_hash)
        .bind(admin_id)
        .execute(&state.pool)
        .await
        .is_err()
    {
        return Redirect::to("/admin/dashboard?error=Database+error");
    }

    Redirect::to("/admin/dashboard?msg=Password+updated")
}