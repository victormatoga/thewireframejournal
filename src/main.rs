use axum::{
    extract::{Form, Multipart, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::{net::SocketAddr, path::Path as StdPath, sync::Arc};
use tera::{Context, Tera};
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, Session, SessionManagerLayer};
use uuid::Uuid;

// --- Models ---

#[derive(Serialize, Deserialize, Clone, Debug, sqlx::FromRow)]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub author: String,
    pub category: String,
    pub image_url: Option<String>,
    pub video_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, sqlx::FromRow)]
pub struct Subscriber {
    pub id: i64,
    pub email: String,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct SubscribeForm {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub struct AppState {
    pub tera: Tera,
    pub db: Pool<Sqlite>,
}

// --- Main Application Entry Point ---

#[tokio::main]
async fn main() {
    tokio::fs::create_dir_all("uploads").await.unwrap();

    // 1. Initialize SQLite Database Pool
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:news.db?mode=rwc".to_string());
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("Failed to connect to SQLite database");

    // Run migrations schema
    let schema = include_str!("../init_db.sql");
    sqlx::query(schema)
        .execute(&db)
        .await
        .expect("Failed to execute database schema initialization");

    // Seed initial admin user if none exists (Username: admin, Password: adminpassword)
    let admin_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM admins")
        .fetch_one(&db)
        .await
        .unwrap_or((0,));

    if admin_count.0 == 0 {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(b"adminpassword", &salt)
            .unwrap()
            .to_string();

        sqlx::query("INSERT INTO admins (username, password_hash) VALUES (?, ?)")
            .bind("admin")
            .bind(password_hash)
            .execute(&db)
            .await
            .ok();
    }

    // 2. Initialize Tera Templates
    let tera = match Tera::new("templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Template parsing error: {}", e);
            std::process::exit(1);
        }
    };

    let shared_state = Arc::new(AppState { tera, db });

    // 3. Setup Session Layer for Admin Authentication
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false); // Set to true in production HTTPS environments

    // 4. Construct Router
    let app = Router::new()
        // Web Frontend Routes
        .route("/", get(render_index))
        .route("/category/:name", get(render_category))
        .route("/search", get(handle_search))
        .route("/article/:id", get(render_article_detail))
        .route("/subscribe", post(handle_subscribe))
        .route("/login", get(render_login).post(handle_login))
        .route("/logout", get(handle_logout))
        
        // Admin Portal Routes (Protected)
        .route("/admin", get(render_admin_dashboard))
        .route("/admin/create", post(handle_create_article))
        .route("/admin/edit/:id", get(render_edit_article).post(handle_update_article))
        .route("/admin/delete/:id", post(handle_delete_article))

        // REST API Endpoints (For Native Mobile Apps / External Services)
        .route("/api/v1/articles", get(api_get_articles))
        .route("/api/v1/articles/:id", get(api_get_article_by_id))
        .route("/api/v1/subscribers", post(api_subscribe))

        // Static Uploads Serving
        .nest_service("/uploads", ServeDir::new("uploads"))
        .layer(session_layer)
        .with_state(shared_state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr_str = format!("0.0.0.0:{}", port);
    println!("Server live on http://{}", addr_str);

    let addr: SocketAddr = addr_str.parse().expect("Invalid socket address format");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// --- Helper Functions ---

async fn is_authenticated(session: &Session) -> bool {
    session.get::<String>("admin_user").await.unwrap_or(None).is_some()
}

async fn save_file_to_storage(name: &str, field: axum::extract::multipart::Field<'_>) -> Option<String> {
    let file_name = field.file_name().map(|s| s.to_string())?;
    if file_name.trim().is_empty() {
        return None;
    }

    let ext = StdPath::new(&file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");

    let new_filename = format!("{}.{}", Uuid::new_v4(), ext);
    let save_path = format!("uploads/{}", new_filename);

    if let Ok(bytes) = field.bytes().await {
        if !bytes.is_empty() && tokio::fs::write(&save_path, bytes).await.is_ok() {
            // If Cloudinary environment variables are configured, upload to Cloud Storage
            if let (Ok(cloud_name), Ok(upload_preset)) = (
                std::env::var("CLOUDINARY_CLOUD_NAME"),
                std::env::var("CLOUDINARY_UPLOAD_PRESET"),
            ) {
                let client = reqwest::Client::new();
                let upload_url = format!("https://api.cloudinary.com/v1_1/{}/auto/upload", cloud_name);
                
                let form = reqwest::multipart::Form::new()
                    .text("upload_preset", upload_preset)
                    .file("file", &save_path)
                    .await;

                if let Ok(form_data) = form {
                    if let Ok(res) = client.post(&upload_url).multipart(form_data).send().await {
                        if let Ok(json) = res.json::<serde_json::Value>().await {
                            if let Some(secure_url) = json.get("secure_url").and_then(|u| u.as_str()) {
                                return Some(secure_url.to_string());
                            }
                        }
                    }
                }
            }
            return Some(format!("/uploads/{}", new_filename));
        }
    }
    None
}

// --- Authentication Handlers ---

async fn render_login(State(state): State<Arc<AppState>>) -> Html<String> {
    let context = Context::new();
    match state.tera.render("login.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(_) => Html("<h1>Login Page (Create templates/login.html)</h1>".to_string()),
    }
}

async fn handle_login(
    State(state): State<Arc<AppState>>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    let result: Option<(String,)> = sqlx::query_as("SELECT password_hash FROM admins WHERE username = ?")
        .bind(&form.username)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

    if let Some((stored_hash,)) = result {
        let parsed_hash = PasswordHash::new(&stored_hash).unwrap();
        if Argon2::default().verify_password(form.password.as_bytes(), &parsed_hash).is_ok() {
            session.insert("admin_user", form.username).await.unwrap();
            return Redirect::to("/admin").into_response();
        }
    }
    Redirect::to("/login?error=invalid_credentials").into_response()
}

async fn handle_logout(session: Session) -> Redirect {
    session.purge().await.ok();
    Redirect::to("/")
}

// --- Web Frontend Handlers ---

async fn render_index(
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let articles: Vec<Article> = sqlx::query_as("SELECT * FROM articles ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let mut context = Context::new();
    context.insert("title", "Home");
    context.insert("articles", &articles);

    if params.contains_key("subscribed") {
        context.insert("subscribed_success", &true);
    }

    match state.tera.render("index.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(err) => Html(format!("Template error: {}", err)),
    }
}

async fn render_category(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let articles: Vec<Article> = sqlx::query_as("SELECT * FROM articles WHERE LOWER(category) = LOWER(?) ORDER BY id DESC")
        .bind(&name)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let mut context = Context::new();
    context.insert("title", &name);
    context.insert("active_category", &format!("Category: {}", name));
    context.insert("articles", &articles);

    match state.tera.render("index.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(err) => Html(format!("Template error: {}", err)),
    }
}

async fn handle_search(
    Query(params): Query<SearchQuery>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let query_str = params.q.unwrap_or_default();
    let search_pattern = format!("%{}%", query_str);

    let articles: Vec<Article> = sqlx::query_as(
        "SELECT * FROM articles WHERE title LIKE ? OR content LIKE ? OR category LIKE ? ORDER BY id DESC",
    )
    .bind(&search_pattern)
    .bind(&search_pattern)
    .bind(&search_pattern)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut context = Context::new();
    context.insert("title", "Search Results");
    context.insert("active_category", &format!("Search Results for: \"{}\"", query_str));
    context.insert("articles", &articles);

    match state.tera.render("index.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(err) => Html(format!("Template error: {}", err)),
    }
}

async fn render_article_detail(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let article: Option<Article> = sqlx::query_as("SELECT * FROM articles WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

    if let Some(article) = article {
        let mut context = Context::new();
        context.insert("article", &article);

        match state.tera.render("article_detail.html", &context) {
            Ok(rendered) => Html(rendered),
            Err(err) => Html(format!("Template error: {}", err)),
        }
    } else {
        Html("<h1 style='text-align:center; margin-top:50px;'>404 - Article Not Found</h1>".to_string())
    }
}

async fn handle_subscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<SubscribeForm>,
) -> Redirect {
    if !form.email.trim().is_empty() {
        sqlx::query("INSERT OR IGNORE INTO subscribers (email, name) VALUES (?, ?)")
            .bind(form.email.trim())
            .bind(form.name)
            .execute(&state.db)
            .await
            .ok();
    }
    Redirect::to("/?subscribed=true")
}

// --- Protected Admin Dashboard Handlers ---

async fn render_admin_dashboard(
    session: Session,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if !is_authenticated(&session).await {
        return Redirect::to("/login").into_response();
    }

    let articles: Vec<Article> = sqlx::query_as("SELECT * FROM articles ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let sub_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM subscribers")
        .fetch_one(&state.db)
        .await
        .unwrap_or((0,));

    let mut context = Context::new();
    context.insert("articles", &articles);
    context.insert("subscribers_count", &sub_count.0);

    match state.tera.render("admin.html", &context) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(err) => Html(format!("Template error: {}", err)).into_response(),
    }
}

async fn handle_create_article(
    session: Session,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !is_authenticated(&session).await {
        return Redirect::to("/login").into_response();
    }

    let mut title = String::new();
    let mut category = String::new();
    let mut author = String::new();
    let mut content = String::new();
    let mut image_url: Option<String> = None;
    let mut video_url: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "image_file" {
            image_url = save_file_to_storage("image_file", field).await;
        } else if name == "video_file" {
            video_url = save_file_to_storage("video_file", field).await;
        } else if let Ok(text) = field.text().await {
            match name.as_str() {
                "title" => title = text,
                "category" => category = text,
                "author" => author = text,
                "content" => content = text,
                _ => {}
            }
        }
    }

    sqlx::query(
        "INSERT INTO articles (title, content, author, category, image_url, video_url) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&title)
    .bind(&content)
    .bind(&author)
    .bind(&category)
    .bind(&image_url)
    .bind(&video_url)
    .execute(&state.db)
    .await
    .ok();

    Redirect::to("/admin").into_response()
}

async fn render_edit_article(
    session: Session,
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if !is_authenticated(&session).await {
        return Redirect::to("/login").into_response();
    }

    let article: Option<Article> = sqlx::query_as("SELECT * FROM articles WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

    if let Some(article) = article {
        let mut context = Context::new();
        context.insert("article", &article);

        match state.tera.render("admin_edit.html", &context) {
            Ok(rendered) => Html(rendered).into_response(),
            Err(err) => Html(format!("Template error: {}", err)).into_response(),
        }
    } else {
        Html("Article not found".to_string()).into_response()
    }
}

async fn handle_update_article(
    session: Session,
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !is_authenticated(&session).await {
        return Redirect::to("/login").into_response();
    }

    let mut title = String::new();
    let mut category = String::new();
    let mut author = String::new();
    let mut content = String::new();
    let mut new_image_url: Option<String> = None;
    let mut new_video_url: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "image_file" {
            new_image_url = save_file_to_storage("image_file", field).await;
        } else if name == "video_file" {
            new_video_url = save_file_to_storage("video_file", field).await;
        } else if let Ok(text) = field.text().await {
            match name.as_str() {
                "title" => title = text,
                "category" => category = text,
                "author" => author = text,
                "content" => content = text,
                _ => {}
            }
        }
    }

    if let Some(img) = new_image_url {
        sqlx::query("UPDATE articles SET image_url = ? WHERE id = ?")
            .bind(img)
            .bind(id)
            .execute(&state.db)
            .await
            .ok();
    }

    if let Some(vid) = new_video_url {
        sqlx::query("UPDATE articles SET video_url = ? WHERE id = ?")
            .bind(vid)
            .bind(id)
            .execute(&state.db)
            .await
            .ok();
    }

    sqlx::query(
        "UPDATE articles SET title = ?, category = ?, author = ?, content = ? WHERE id = ?",
    )
    .bind(title)
    .bind(category)
    .bind(author)
    .bind(content)
    .bind(id)
    .execute(&state.db)
    .await
    .ok();

    Redirect::to("/admin").into_response()
}

async fn handle_delete_article(
    session: Session,
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if !is_authenticated(&session).await {
        return Redirect::to("/login").into_response();
    }

    sqlx::query("DELETE FROM articles WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .ok();

    Redirect::to("/admin").into_response()
}

// --- REST API Endpoints ---

async fn api_get_articles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Article>>, StatusCode> {
    let articles: Vec<Article> = sqlx::query_as("SELECT * FROM articles ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(articles))
}

async fn api_get_article_by_id(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Article>, StatusCode> {
    let article: Article = sqlx::query_as("SELECT * FROM articles WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(article))
}

async fn api_subscribe(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SubscribeForm>,
) -> StatusCode {
    if payload.email.trim().is_empty() {
        return StatusCode::BAD_REQUEST;
    }

    let res = sqlx::query("INSERT OR IGNORE INTO subscribers (email, name) VALUES (?, ?)")
        .bind(payload.email.trim())
        .bind(payload.name)
        .execute(&state.db)
        .await;

    match res {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}