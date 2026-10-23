//! `lichtung-server` binary: `lichtung-server` runs the server,
//! `lichtung-server healthcheck` checks a running one (exit code 0/1).

use std::process::ExitCode;

use lichtung_server::Config;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
Aufruf:
  lichtung-server               Server starten (Konfiguration über LICHTUNG_*-Variablen)
  lichtung-server healthcheck   /healthz des lokalen Servers prüfen (Exit-Code 0 = gesund)
  lichtung-server reset-secret-key
                                nach Verlust von LICHTUNG_SECRET_KEY: gespeicherte Zugangsdaten
                                verwerfen und den neuen Schlüssel eintragen (Server vorher stoppen)
  lichtung-server --version     Version ausgeben";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => serve().await,
        ["healthcheck"] => healthcheck().await,
        ["reset-secret-key"] => reset_secret_key(),
        ["--version" | "-V"] => {
            println!("lichtung-server {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ["--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

async fn serve() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match lichtung_server::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = ?e, "server stopped with an error");
            // Also on stderr, in German, for whoever starts the container.
            eprintln!("Lichtung-Server beendet: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn reset_secret_key() -> ExitCode {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match lichtung_server::accounts::reset_secret_key(&config) {
        Ok(n) => {
            println!(
                "Schlüssel eingetragen. Gespeicherte Zugangsdaten verworfen: {n} Konto/Konten \
                 in der App neu verbinden."
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Zurücksetzen fehlgeschlagen: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn healthcheck() -> ExitCode {
    // Only the bind address: the check must not depend on the secrets.
    let bind = match Config::bind_from_env() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match lichtung_server::healthcheck::check(bind).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ungesund: {e}");
            ExitCode::FAILURE
        }
    }
}
