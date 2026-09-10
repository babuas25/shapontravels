use shapontravels_api::{
    AppState, MIGRATOR, config::Config, connect, router_with_search_limits, schema_ready,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();
    if let Err(message) = run().await {
        // Errors deliberately omit underlying driver/config values, which may contain secrets.
        tracing::error!("{message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    match dotenvy::dotenv() {
        Ok(_) => (),
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err("could not parse .env".into()),
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() > 1
        || args
            .first()
            .is_some_and(|v| !["serve", "migrate", "bootstrap-admin"].contains(&v.as_str()))
    {
        return Err("usage: shapontravels-api [serve|migrate|bootstrap-admin]".into());
    }
    let config = Config::from_env()?;
    let pool = connect(&config)
        .await
        .map_err(|_| "database connection failed")?;
    if args.first().is_some_and(|v| v == "migrate") {
        MIGRATOR
            .run(&pool)
            .await
            .map_err(|_| "database migration failed")?;
        tracing::info!("database migrations applied");
        pool.close().await;
        return Ok(());
    }
    if !schema_ready(&pool).await {
        return Err("database schema is not current; run migrate".into());
    }
    if args.first().is_some_and(|v| v == "bootstrap-admin") {
        use std::io::{self, Write};
        print!("Super Admin username: ");
        io::stdout().flush().map_err(|_| "terminal unavailable")?;
        let mut username = String::new();
        io::stdin()
            .read_line(&mut username)
            .map_err(|_| "could not read username")?;
        let password = rpassword::prompt_password("Password (12–256 bytes): ")
            .map_err(|_| "could not read password")?;
        let confirm = rpassword::prompt_password("Confirm password: ")
            .map_err(|_| "could not read password")?;
        if password != confirm {
            return Err("passwords do not match".into());
        }
        shapontravels_api::auth::bootstrap(&pool, username.trim().into(), password)
            .await
            .map_err(|e| e.1)?;
        tracing::info!("Super Admin created; bootstrap is now closed");
        pool.close().await;
        return Ok(());
    }
    let mut suppliers = std::collections::HashMap::new();
    for supplier in config.suppliers {
        let id = supplier.id.to_string();
        let currency = supplier.currency.clone();
        if supplier.email.is_some() && supplier.password.is_some() {
            let adapter = shapontravels_api::supplier::SupplierAdapter::new(
                supplier,
                std::time::Duration::from_secs(120),
            )
            .map_err(|_| "supplier configuration invalid")?;
            suppliers.insert(
                id,
                shapontravels_api::search::ConfiguredSupplier {
                    transport: std::sync::Arc::new(adapter),
                    currency,
                },
            );
        }
    }
    let state = AppState {
        suppliers: std::sync::Arc::new(suppliers),
        pool: pool.clone(),
        environment: config.environment.clone(),
        db_timeout: config.db_timeout,
    };
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|_| "could not bind HTTP listener")?;
    tracing::info!(environment = %config.environment, bind = %config.bind, "API started");
    tracing::info!(
        max_active = config.search_limits.max_active,
        max_queued = config.search_limits.max_queued,
        queue_wait_ms = config.search_limits.wait.as_millis() as u64,
        "Search admission configured"
    );
    let (cleanup_stop, cleanup_receiver) = tokio::sync::oneshot::channel();
    let cleanup = tokio::spawn(shapontravels_api::cleanup::run(
        pool.clone(),
        cleanup_receiver,
    ));
    let result = axum::serve(
        listener,
        router_with_search_limits(state, config.search_limits),
    )
    .with_graceful_shutdown(shutdown())
    .await;
    let _ = cleanup_stop.send(());
    let _ = cleanup.await;
    result.map_err(|_| "HTTP server failed")?;
    pool.close().await;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
