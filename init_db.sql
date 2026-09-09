CREATE TABLE IF NOT EXISTS articles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    author TEXT NOT NULL,
    category TEXT NOT NULL,
    image_url TEXT,
    video_url TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS subscribers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT UNIQUE NOT NULL,
    name TEXT
);

CREATE TABLE IF NOT EXISTS admins (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL
);
-- Admin Credentials Table
CREATE TABLE IF NOT EXISTS admins (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Article Views & Engagement
CREATE TABLE IF NOT EXISTS article_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    article_id INTEGER NOT NULL,
    ip_address TEXT,
    viewed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(article_id) REFERENCES articles(id)
);

-- Subscribers
CREATE TABLE IF NOT EXISTS subscribers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT UNIQUE NOT NULL,
    status TEXT DEFAULT 'active', -- active, unsubscribed
    subscribed_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Monetization & Ad Performance
CREATE TABLE IF NOT EXISTS ad_impressions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ad_placement TEXT NOT NULL,
    impressions INTEGER DEFAULT 0,
    clicks INTEGER DEFAULT 0,
    estimated_revenue REAL DEFAULT 0.0,
    recorded_date DATE DEFAULT CURRENT_DATE
);