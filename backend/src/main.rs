use meta_leads_backend::{app, config::Config, push, store::Store};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    // `meta-leads-backend vapid` prints a fresh VAPID key pair for Web Push.
    if std::env::args().nth(1).as_deref() == Some("vapid") {
        let (public, private) = push::generate_vapid();
        println!("VAPID_PUBLIC_KEY={public}\nVAPID_PRIVATE_KEY={private}");
        return Ok(());
    }

    let config = Config::from_env()?;
    for (missing, msg) in [
        (
            config.app_secret.is_empty(),
            "META_APP_SECRET is empty: webhook POSTs will be rejected.",
        ),
        (
            config.verify_token.is_empty(),
            "META_VERIFY_TOKEN is empty: Meta's webhook handshake will fail.",
        ),
        (
            config.page_tokens.is_empty() && config.user_token.is_empty(),
            "No Page token configured (META_PAGE_TOKENS, META_PAGE_ID + META_PAGE_ACCESS_TOKEN or META_USER_TOKEN): leads will only have webhook ids.",
        ),
        (
            config.vapid_public_key.is_empty(),
            "VAPID keys missing: run `meta-leads-backend vapid` to enable push.",
        ),
    ] {
        if missing {
            tracing::warn!("{msg}");
        }
    }

    let store = Store::open(&config.data_file).await?;
    let addr = config.bind_addr.clone();
    let state = app::AppState::new(config, store)?;
    state.resolve_pages().await;
    for page in state.pages.list().await {
        tracing::info!(
            "reading leads from Page {} ({})",
            page.id,
            page.name.as_deref().unwrap_or("name unknown")
        );
    }
    state.resolve_pages().await;
    for page in state.pages.list().await {
        tracing::info!(
            "reading leads from Page {} ({})",
            page.id,
            page.name.as_deref().unwrap_or("name unknown")
        );
    }
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("meta-leads listening on {addr}");
    axum::serve(listener, app::router(state)).await?;
    Ok(())
}
