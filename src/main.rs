use axum::{
    extract::{Form, Multipart, Path, Query, State},
    response::{Html, Redirect},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{path::Path as StdPath, sync::Arc};
use tokio::{fs, sync::RwLock};
use tera::{Context, Tera};
use tower_http::services::ServeDir;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Article {
    pub id: u32,
    pub title: String,
    pub content: String,
    pub author: String,
    pub category: String,
    pub image_url: Option<String>,
    pub video_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Subscriber {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct SubscribeForm {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub struct AppState {
    pub tera: Tera,
    pub articles: RwLock<Vec<Article>>,
    pub subscribers: RwLock<Vec<Subscriber>>,
}

#[tokio::main]
async fn main() {
    fs::create_dir_all("uploads").await.unwrap();

    let mut tera = match Tera::new("templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Template parsing error: {}", e);
            std::process::exit(1);
        }
    };

    tera.full_reload().unwrap();

    let initial_articles = vec![
        Article {
            id: 1,
            title: String::from("Rust Web Architecture Takes Off"),
            content: String::from("Building web engines using Rust, Axum, and Tokio delivers unmatched speed."),
            author: String::from("Victor Matoga"),
            category: String::from("Business"),
            image_url: None,
            video_url: None,
        },
    ];

    let shared_state = Arc::new(AppState {
        tera,
        articles: RwLock::new(initial_articles),
        subscribers: RwLock::new(Vec::new()),
    });

    let app = Router::new()
        .route("/", get(render_index))
        .route("/category/:name", get(render_category))
        .route("/search", get(handle_search))
        .route("/article/:id", get(render_article_detail))
        .route("/subscribe", post(handle_subscribe))
        .route("/admin", get(render_admin_dashboard))
        .route("/admin/create", post(handle_create_article))
        .route("/admin/edit/:id", get(render_edit_article).post(handle_update_article))
        .route("/admin/delete/:id", post(handle_delete_article))
        .nest_service("/uploads", ServeDir::new("uploads"))
        .with_state(shared_state);

    let addr = "127.0.0.1:3000";
    println!("Server running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_subscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<SubscribeForm>,
) -> Redirect {
    if !form.email.trim().is_empty() {
        let mut subs = state.subscribers.write().await;
        if !subs.iter().any(|s| s.email == form.email) {
            subs.push(Subscriber {
                email: form.email.clone(),
                name: form.name,
            });
            println!("New subscriber added: {}", form.email);
        }
    }
    Redirect::to("/?subscribed=true")
}

async fn notify_subscribers(state: &AppState, article_title: &str) {
    let subs = state.subscribers.read().await;
    for sub in subs.iter() {
        println!(
            "[NOTIFICATION SENT] Email to: {} -> New story published: '{}'",
            sub.email, article_title
        );
    }
}

async fn render_index(
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let articles = state.articles.read().await;
    let mut context = Context::new();
    context.insert("title", "Home");
    context.insert("articles", &*articles);

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
    let articles = state.articles.read().await;
    let filtered_articles: Vec<Article> = articles
        .iter()
        .filter(|a| a.category.to_lowercase() == name.to_lowercase())
        .cloned()
        .collect();

    let mut context = Context::new();
    context.insert("title", &name);
    context.insert("active_category", &format!("Category: {}", name));
    context.insert("articles", &filtered_articles);

    match state.tera.render("index.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(err) => Html(format!("Template error: {}", err)),
    }
}

async fn handle_search(
    Query(params): Query<SearchQuery>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let query_str = params.q.unwrap_or_default().to_lowercase();
    let articles = state.articles.read().await;

    let matching_articles: Vec<Article> = articles
        .iter()
        .filter(|a| {
            a.title.to_lowercase().contains(&query_str)
                || a.content.to_lowercase().contains(&query_str)
                || a.category.to_lowercase().contains(&query_str)
        })
        .cloned()
        .collect();

    let mut context = Context::new();
    context.insert("title", "Search Results");
    context.insert("active_category", &format!("Search Results for: \"{}\"", query_str));
    context.insert("articles", &matching_articles);

    match state.tera.render("index.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(err) => Html(format!("Template error: {}", err)),
    }
}

async fn render_article_detail(
    Path(id): Path<u32>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let articles = state.articles.read().await;
    if let Some(article) = articles.iter().find(|a| a.id == id) {
        let mut context = Context::new();
        context.insert("article", article);

        match state.tera.render("article_detail.html", &context) {
            Ok(rendered) => Html(rendered),
            Err(err) => Html(format!("Template error: {}", err)),
        }
    } else {
        Html(String::from("<h1 style='text-align:center; margin-top:50px;'>404 - Article Not Found</h1>"))
    }
}

async fn render_admin_dashboard(State(state): State<Arc<AppState>>) -> Html<String> {
    let articles = state.articles.read().await;
    let subs = state.subscribers.read().await;
    let mut context = Context::new();
    context.insert("articles", &*articles);
    context.insert("subscribers_count", &subs.len());

    match state.tera.render("admin.html", &context) {
        Ok(rendered) => Html(rendered),
        Err(err) => Html(format!("Template error: {}", err)),
    }
}

async fn handle_create_article(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Redirect {
    let mut title = String::new();
    let mut category = String::new();
    let mut author = String::new();
    let mut content = String::new();
    let mut image_url: Option<String> = None;
    let mut video_url: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "image_file" || name == "video_file" {
            let file_name = field.file_name().map(|s| s.to_string());
            if let Some(fname) = file_name {
                if !fname.trim().is_empty() {
                    let ext = StdPath::new(&fname)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("bin");
                    
                    let new_filename = format!("{}.{}", Uuid::new_v4(), ext);
                    let save_path = format!("uploads/{}", new_filename);

                    if let Ok(bytes) = field.bytes().await {
                        if !bytes.is_empty() && fs::write(&save_path, bytes).await.is_ok() {
                            let web_path = format!("/uploads/{}", new_filename);
                            if name == "image_file" {
                                image_url = Some(web_path);
                            } else {
                                video_url = Some(web_path);
                            }
                        }
                    }
                }
            }
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

    let mut articles = state.articles.write().await;
    let new_id = articles.iter().map(|a| a.id).max().unwrap_or(0) + 1;

    let new_article = Article {
        id: new_id,
        title: title.clone(),
        category,
        author,
        content,
        image_url,
        video_url,
    };

    articles.push(new_article);

    // Notify all subscribers of the new article
    notify_subscribers(&state, &title).await;

    Redirect::to("/admin")
}

async fn render_edit_article(
    Path(id): Path<u32>,
    State(state): State<Arc<AppState>>,
) -> Html<String> {
    let articles = state.articles.read().await;
    if let Some(article) = articles.iter().find(|a| a.id == id) {
        let mut context = Context::new();
        context.insert("article", article);

        match state.tera.render("admin_edit.html", &context) {
            Ok(rendered) => Html(rendered),
            Err(err) => Html(format!("Template error: {}", err)),
        }
    } else {
        Html(String::from("Article not found"))
    }
}

async fn handle_update_article(
    Path(id): Path<u32>,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Redirect {
    let mut title = String::new();
    let mut category = String::new();
    let mut author = String::new();
    let mut content = String::new();
    let mut new_image_url: Option<String> = None;
    let mut new_video_url: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "image_file" || name == "video_file" {
            let file_name = field.file_name().map(|s| s.to_string());
            if let Some(fname) = file_name {
                if !fname.trim().is_empty() {
                    let ext = StdPath::new(&fname)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("bin");
                    
                    let new_filename = format!("{}.{}", Uuid::new_v4(), ext);
                    let save_path = format!("uploads/{}", new_filename);

                    if let Ok(bytes) = field.bytes().await {
                        if !bytes.is_empty() && fs::write(&save_path, bytes).await.is_ok() {
                            let web_path = format!("/uploads/{}", new_filename);
                            if name == "image_file" {
                                new_image_url = Some(web_path);
                            } else {
                                new_video_url = Some(web_path);
                            }
                        }
                    }
                }
            }
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

    let mut articles = state.articles.write().await;
    if let Some(article) = articles.iter_mut().find(|a| a.id == id) {
        article.title = title.clone();
        article.category = category;
        article.author = author;
        article.content = content;
        if new_image_url.is_some() {
            article.image_url = new_image_url;
        }
        if new_video_url.is_some() {
            article.video_url = new_video_url;
        }
    }

    notify_subscribers(&state, &format!("(Updated) {}", title)).await;

    Redirect::to("/admin")
}

async fn handle_delete_article(
    Path(id): Path<u32>,
    State(state): State<Arc<AppState>>,
) -> Redirect {
    let mut articles = state.articles.write().await;
    articles.retain(|a| a.id != id);
    Redirect::to("/admin")
}