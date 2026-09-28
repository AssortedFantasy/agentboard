use agentboard::{
    app, cli, db,
    model::{Output, Request},
    render, web,
};
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    io::{self, Write},
    time::Instant,
};

fn main() {
    let raw: Vec<_> = std::env::args_os().collect();
    let invocation = match cli::parse_from(raw.clone()) {
        Ok(invocation) => invocation,
        Err(error) => {
            if let Some(clap) = error.downcast_ref::<clap::Error>() {
                let _ = clap.print();
                if clap.exit_code() != 0 {
                    log_parse_failure(&raw, &error);
                }
                std::process::exit(clap.exit_code());
            }
            log_parse_failure(&raw, &error);
            eprintln!("error: {error:#}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(invocation) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run(mut invocation: cli::Invocation) -> Result<()> {
    if invocation.request.command == "serve" {
        return web::serve_as(
            &invocation.database,
            invocation.request.args["bind"]
                .as_str()
                .unwrap_or("127.0.0.1:8080"),
            &invocation.actor,
        );
    }
    let started = Instant::now();
    let mut conn = db::open(&invocation.database)?;
    apply_config(&conn, &mut invocation)?;
    let result = (|| -> Result<Output> {
        let output = app::execute(&mut conn, &invocation.actor, &invocation.request)?;
        let rendered = render::render(
            &output,
            &invocation.format,
            invocation.compact,
            invocation.max_bytes,
        )?;
        {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(rendered.text.as_bytes())
                .context("write output; no observation receipts were applied")?;
            stdout
                .flush()
                .context("flush output; no observation receipts were applied")?;
        }
        app::acknowledge(&mut conn, &invocation.actor, &rendered).context(
            "output delivered but receipt persistence failed; it may repeat on the next read",
        )?;
        Ok(output)
    })();
    let logged = app::log_command(
        &conn,
        &invocation.actor,
        &invocation.request,
        &result,
        started.elapsed().as_millis().min(i64::MAX as u128) as i64,
    );
    if let Err(error) = logged {
        eprintln!("warning: could not persist command log: {error:#}");
    }
    result.map(|_| ())
}

fn apply_config(conn: &rusqlite::Connection, invocation: &mut cli::Invocation) -> Result<()> {
    let args = &mut invocation.request.args;
    for key in ["limit", "max_bytes", "format", "compact"] {
        if !args[format!("_explicit_{key}")].as_bool().unwrap_or(false)
            && let Some(value) = db::config(conn, key)?
        {
            match key {
                "max_bytes" => {
                    invocation.max_bytes =
                        value.as_u64().context("invalid max_bytes configuration")? as usize
                }
                "format" => {
                    invocation.format = value
                        .as_str()
                        .context("invalid format configuration")?
                        .into()
                }
                "compact" => {
                    invocation.compact = value.as_bool().context("invalid compact configuration")?
                }
                _ => args[key] = value,
            }
        }
    }
    for key in ["query_ms", "poll_ms"] {
        if !args[format!("_explicit_{key}")].as_bool().unwrap_or(false)
            && let Some(value) = db::config(conn, key)?
        {
            args[key] = value;
        }
    }
    args["max_bytes"] = json!(invocation.max_bytes);
    args["compact"] = json!(invocation.compact);
    Ok(())
}

fn log_parse_failure(raw: &[std::ffi::OsString], error: &anyhow::Error) {
    if raw.len() < 3 {
        return;
    }
    let path = std::path::Path::new(&raw[1]);
    if !path.is_file() {
        return;
    }
    if let Ok(conn) = db::open(path) {
        let request = Request {
            command: "cli.parse".into(),
            args: json!({"argument_count":raw.len()-1}),
        };
        let result = Err(anyhow::anyhow!("{error}"));
        let _ = app::log_command(&conn, &raw[2].to_string_lossy(), &request, &result, 0);
    }
}
